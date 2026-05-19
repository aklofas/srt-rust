//! PSI section dispatch + PAT/PMT topology tracking.
//!
//! Hosts 5 helper methods on `Demuxer`:
//!
//! - `handle_psi` — router from `process_packet` into PAT vs PMT
//!   continuation logic, including strict-mode CC-discontinuity drop.
//! - `handle_pat_section` — full PAT parse + program-tracker diff
//!   (programs added / removed across PAT version bumps).
//! - `handle_pmt_section` — full PMT parse + per-program PMT version
//!   bump + stream-binding update + KLV link inference + descriptor
//!   diagnostics + cross-program PID collision policy.
//! - `build_program_map` — composes a `ProgramMap` event from a parsed
//!   PMT after the topology has settled.
//! - `klv_mismatch_insert` — per-program coalesce-set helper for KLV
//!   stream-type-mismatch event deduplication.
//!
//! All items are `pub(super)` — invisible outside `mpegts::demux`.

use crate::error::DemuxError;
use crate::mpegts::common::StreamTypeCode;
use crate::mpegts::demux::event::{
    DemuxEvent, KlvLink, LinkSource, NonConformantIssue, ProgramMap, StreamId, StreamInfo,
    StreamKind, VideoCodec,
};
use crate::mpegts::demux::psi::{Pmt, PsiParseError, parse_pat, parse_pmt};
use crate::mpegts::demux::psi_assembler::AssemblerError;
use crate::mpegts::demux::types::ProgramTracker;
use std::collections::HashSet;

impl super::demuxer::Demuxer {
    pub(super) fn handle_psi(
        &mut self,
        pid: u16,
        payload: &[u8],
        pusi: bool,
        is_pat: bool,
        cc_jumped: bool,
    ) -> Result<(), DemuxError> {
        // Strict mode: drop the partial section if a continuation packet
        // arrives with a CC jump (matches ffmpeg `mpegts.c:3118-3142`).
        // Lenient mode (opt-in via DemuxerConfig::lenient_psi_reassembly)
        // preserves today's permissive behavior — bytes are accumulated
        // regardless, either passing CRC by luck or surfacing as
        // PsiChecksumMismatch.
        if !pusi && cc_jumped && !self.options.lenient_psi_reassembly {
            if let Some(assembler) = self.psi_assemblers.get_mut(&pid) {
                assembler.reset();
            }
            let (expected, observed) = match self.last_psi_cc_jump.take() {
                Some(pair) => pair,
                None => {
                    // Invariant violation: `check_continuity` must populate
                    // `last_psi_cc_jump` before reaching this arm. Defense-in-depth:
                    // drop the event silently rather than panicking, so a future
                    // refactor that decouples `check_continuity` from `handle_psi`
                    // can't crash the demuxer in production. `debug_assert!` still
                    // catches regressions in test runs.
                    debug_assert!(
                        false,
                        "check_continuity invariant: last_psi_cc_jump should be populated"
                    );
                    return Ok(());
                }
            };
            let stream = self.lookup_stream(pid).unwrap_or(StreamId {
                pid,
                kind: StreamKind::Unknown(0),
                // program_number unavailable — PSI PID (PAT/PMT) is not owned by a program
                program_number: 0,
            });
            self.queue_nonconformant(
                stream,
                NonConformantIssue::PsiCcDiscontinuity {
                    pid,
                    expected,
                    observed,
                },
            );
            return Ok(());
        }

        let assembler = self.psi_assemblers.entry(pid).or_default();

        let result = if pusi {
            // First byte after pointer_field marks where the section starts.
            if payload.is_empty() {
                return Ok(());
            }
            let pointer_field = payload[0] as usize;
            if 1 + pointer_field > payload.len() {
                return Ok(());
            }
            assembler.start_new_section(&payload[1 + pointer_field..])
        } else {
            assembler.append_continuation(payload)
        };

        let section = match result {
            Ok(Some(section)) => section,
            Ok(None) => return Ok(()),
            Err(AssemblerError::Overflow { observed_len })
            | Err(AssemblerError::DeclaredTooLong {
                declared_len: observed_len,
            }) => {
                // Cap fired — partial section discarded by the assembler. Surface
                // the overflow as a NonConformant event keyed to this PSI PID so
                // the caller can detect the DoS attempt without losing the
                // receive loop.
                let stream = self.lookup_stream(pid).unwrap_or(StreamId {
                    pid,
                    kind: StreamKind::Unknown(0),
                    // program_number unavailable — PSI PID (PAT/PMT) is not owned by a program
                    program_number: 0,
                });
                self.queue_nonconformant(
                    stream,
                    NonConformantIssue::PsiOverlongSection { pid, observed_len },
                );
                return Ok(());
            }
        };

        if is_pat {
            self.handle_pat_section(&section);
        } else {
            self.handle_pmt_section(pid, &section);
        }
        Ok(())
    }

    pub(super) fn handle_pat_section(&mut self, section: &[u8]) {
        let pat = match parse_pat(section) {
            Ok(p) => p,
            Err(PsiParseError::CrcMismatch { .. }) => {
                self.queue_nonconformant(
                    StreamId {
                        pid: 0x0000,
                        kind: StreamKind::Unknown(0),
                        // program_number unavailable — PAT PID is not owned by a program
                        program_number: 0,
                    },
                    NonConformantIssue::PsiChecksumMismatch { pid: 0x0000 },
                );
                return;
            }
            Err(PsiParseError::MultiSectionUnsupported {
                table_id,
                last_section_number,
            }) => {
                self.queue_nonconformant(
                    StreamId {
                        pid: 0x0000,
                        kind: StreamKind::Unknown(0),
                        // program_number unavailable — PAT PID is not owned by a program
                        program_number: 0,
                    },
                    NonConformantIssue::PsiMultiSectionUnsupported {
                        pid: 0x0000,
                        table_id,
                        last_section_number,
                    },
                );
                return;
            }
            Err(_) => return,
        };
        // Same version — nothing changed, skip the diff.
        if Some(pat.version) == self.pat_version {
            return;
        }
        self.pat_version = Some(pat.version);

        // Build the set of PMT PIDs in the new PAT, skipping program 0 (NIT).
        let new_pmt_pids: HashSet<u16> = pat
            .programs
            .iter()
            .filter(|e| e.program_number != 0)
            .map(|e| e.pid)
            .collect();

        // Drop trackers for programs that disappeared from this PAT version.
        let removed: Vec<u16> = self
            .programs
            .keys()
            .copied()
            .filter(|pid| !new_pmt_pids.contains(pid))
            .collect();
        for pmt_pid in removed {
            if let Some(tracker) = self.programs.remove(&pmt_pid) {
                // Remove the PES stream-kind entries owned by this program.
                for stream in &tracker.streams {
                    self.stream_kind_by_pid.remove(&stream.pid);
                    self.pid_to_program.remove(&stream.pid);
                }
                // Free the PSI assembly buffer for this PMT PID.
                self.psi_assemblers.remove(&pmt_pid);
            }
        }

        // Add empty trackers for programs that are new in this PAT version.
        // PMT contents will populate them when handle_pmt_section fires.
        for entry in &pat.programs {
            if entry.program_number == 0 {
                continue; // program 0 = Network PID, not a real program
            }
            self.programs
                .entry(entry.pid)
                .or_insert_with(|| ProgramTracker {
                    program_number: entry.program_number,
                    pmt_pid: entry.pid,
                    pmt_version: None,
                    pcr_pid: None,
                    streams: Vec::new(),
                    klv_mismatch_coalesce: HashSet::new(),
                });
            // Seed the PSI assembler for this PMT PID so handle_psi can
            // accumulate bytes without a separate "first packet" init step.
            self.psi_assemblers.entry(entry.pid).or_default();
        }
    }

    pub(super) fn handle_pmt_section(&mut self, pmt_pid: u16, section: &[u8]) {
        // The PMT PID itself is owned by a program (the one this PMT describes).
        // PAT pre-populates `self.programs` keyed by PMT PID, so we can resolve
        // the program_number here even when the PMT body fails to parse.
        let pmt_program_number = self
            .programs
            .get(&pmt_pid)
            .map(|t| t.program_number)
            .unwrap_or(0);
        let pmt = match parse_pmt(section) {
            Ok(p) => p,
            Err(PsiParseError::CrcMismatch { .. }) => {
                self.queue_nonconformant(
                    StreamId {
                        pid: pmt_pid,
                        kind: StreamKind::Unknown(0),
                        program_number: pmt_program_number,
                    },
                    NonConformantIssue::PsiChecksumMismatch { pid: pmt_pid },
                );
                return;
            }
            Err(PsiParseError::MultiSectionUnsupported {
                table_id,
                last_section_number,
            }) => {
                self.queue_nonconformant(
                    StreamId {
                        pid: pmt_pid,
                        kind: StreamKind::Unknown(0),
                        program_number: pmt_program_number,
                    },
                    NonConformantIssue::PsiMultiSectionUnsupported {
                        pid: pmt_pid,
                        table_id,
                        last_section_number,
                    },
                );
                return;
            }
            Err(_) => return,
        };

        // Look up the tracker — must exist if PAT pre-populated us.
        let program_number = match self.programs.get(&pmt_pid) {
            Some(t) => t.program_number,
            None => return, // PMT arriving on PID not in PAT — drop.
        };

        // Dedup: re-emit only if version changed or first ever.
        if let Some(tracker) = self.programs.get(&pmt_pid) {
            if Some(pmt.version) == tracker.pmt_version {
                return;
            }
        }

        // Fresh PMT version — clear the per-PID guard for SubtitleMissingDescriptor
        // and Av1RegistrationMalformed emission so that a new PMT version
        // re-fires the warning if the descriptor is still absent / still
        // malformed. Only PIDs owned by *this* PMT are dropped to leave any
        // other program's state intact.
        for s in &pmt.streams {
            self.subtitle_missing_descriptor_emitted
                .remove(&s.elementary_pid);
            self.av1_registration_malformed_emitted
                .remove(&s.elementary_pid);
            self.subtitle_descriptor_ambiguous_emitted
                .remove(&s.elementary_pid);
        }

        // Build StreamInfo list + check cross-program PID collisions.
        // Collect work to do before mutating self — satisfies borrow checker.
        let mut stream_infos: Vec<StreamInfo> = Vec::with_capacity(pmt.streams.len());
        let mut kind_inserts: Vec<(u16, StreamKind)> = Vec::new();
        let mut collision_issues: Vec<(StreamId, NonConformantIssue)> = Vec::new();
        let mut subtitle_missing: Vec<(u16, StreamKind)> = Vec::new();
        let mut av1_malformed: Vec<(u16, StreamKind)> = Vec::new();
        let mut subtitle_ambiguous: Vec<(u16, StreamKind, Vec<u8>)> = Vec::new();

        for s in &pmt.streams {
            let (kind, _declared_link) = self.get_stream_kind(s.elementary_pid, s);

            // Cross-program PID collision check: scan all other programs'
            // stream lists. First-program-wins — skip this PID if already owned.
            let other_prog = self
                .programs
                .iter()
                .find(|(other_pmt_pid, t)| {
                    **other_pmt_pid != pmt_pid
                        && t.streams.iter().any(|st| st.pid == s.elementary_pid)
                })
                .map(|(_, t)| t.program_number);

            if let Some(other_program_number) = other_prog {
                collision_issues.push((
                    StreamId {
                        pid: s.elementary_pid,
                        kind: StreamKind::Unknown(0),
                        // The PID is first-program-wins owned by `other_program_number`
                        // (the existing binding); this StreamId surfaces the collision
                        // attempted by *this* PMT (`program_number`), so we tag it
                        // with the attempting program.
                        program_number,
                    },
                    NonConformantIssue::PidReusedAcrossPrograms {
                        pid: s.elementary_pid,
                        programs: [other_program_number, program_number],
                    },
                ));
                continue; // Skip this stream — first-program-wins.
            }

            // Subtitle-resolved PIDs without a recognized subtitle descriptor
            // (subtitling/teletext/VTTC/GA94) are non-conformant — most often
            // because a `treat_as` override forced StreamKind::Subtitle on a
            // PID whose PMT entry doesn't carry the corresponding tag.
            if matches!(kind, StreamKind::Subtitle(_))
                && !super::pmt_classify::has_recognized_subtitle_descriptor(&s.descriptors)
            {
                subtitle_missing.push((s.elementary_pid, kind));
            }

            // stream_type 0x06 entries that contain a Registration descriptor
            // whose body looks like a truncated AV01 attempt (starts with
            // "AV", < 4 bytes total) — fires only when classify_0x06 didn't
            // already return Video(Av1), i.e. the malformed registration
            // didn't match `b"AV01"` exactly. Outer length-vs-buffer overflow
            // is already caught by walk_descriptors as DescriptorLoopOverflow;
            // this is the in-bounds variant.
            if s.stream_type == 0x06
                && !matches!(kind, StreamKind::Video(VideoCodec::Av1))
                && super::pmt_classify::is_malformed_av1_registration(&s.descriptors)
            {
                av1_malformed.push((s.elementary_pid, kind));
            }

            // stream_type 0x06 entries with more than one recognized
            // subtitle/KLV codec marker — classification cascade still
            // picks the highest-priority match (subtitling > teletext >
            // VTTC > GA94 > KLVA), but the ambiguity is surfaced for
            // diagnostics. Only checked on 0x06 since the other stream
            // types disambiguate by stream_type alone. AV1 wins
            // exclusively (binding §2.1) so AV01 alongside other markers
            // is not flagged here — classify_0x06 returned Video(Av1).
            if s.stream_type == 0x06 && !matches!(kind, StreamKind::Video(VideoCodec::Av1)) {
                let (_, ambiguous_tags) =
                    super::pmt_classify::classify_0x06_with_ambiguity(&s.descriptors);
                if !ambiguous_tags.is_empty() {
                    subtitle_ambiguous.push((s.elementary_pid, kind, ambiguous_tags));
                }
            }

            stream_infos.push(StreamInfo {
                pid: s.elementary_pid,
                stream_type: StreamTypeCode::from_byte(s.stream_type),
                kind,
                program_number,
                raw_descriptors: s.descriptors.clone(),
            });
            kind_inserts.push((s.elementary_pid, kind));
        }

        // Emit collision NonConformant events.
        for (stream_id, issue) in collision_issues {
            self.queue_nonconformant(stream_id, issue);
        }

        // Emit SubtitleMissingDescriptor once per PID per fresh PMT version.
        for (pid, kind) in subtitle_missing {
            if self.subtitle_missing_descriptor_emitted.insert(pid) {
                self.queue_nonconformant(
                    StreamId {
                        pid,
                        kind,
                        program_number,
                    },
                    NonConformantIssue::SubtitleMissingDescriptor { pid },
                );
            }
        }

        // Emit Av1RegistrationMalformed once per PID per fresh PMT version.
        for (pid, kind) in av1_malformed {
            if self.av1_registration_malformed_emitted.insert(pid) {
                self.queue_nonconformant(
                    StreamId {
                        pid,
                        kind,
                        program_number,
                    },
                    NonConformantIssue::Av1RegistrationMalformed { pid },
                );
            }
        }

        // Emit SubtitleDescriptorAmbiguous once per PID per fresh PMT version.
        for (pid, kind, tags) in subtitle_ambiguous {
            if self.subtitle_descriptor_ambiguous_emitted.insert(pid) {
                self.queue_nonconformant(
                    StreamId {
                        pid,
                        kind,
                        program_number,
                    },
                    NonConformantIssue::SubtitleDescriptorAmbiguous { pid, tags },
                );
            }
        }

        // Update stream_kind_by_pid for all non-colliding streams.
        for (pid, kind) in kind_inserts {
            self.stream_kind_by_pid.insert(pid, kind);
        }

        // Build klv_links from the accepted streams.
        let prog_map = self.build_program_map(&pmt, program_number, &stream_infos);

        // Update tracker.
        let tracker = self.programs.get_mut(&pmt_pid).expect("checked above");
        tracker.pmt_version = Some(pmt.version);
        tracker.pcr_pid = Some(pmt.pcr_pid);
        // Remove stale pid_to_program entries for PIDs previously owned by
        // this program (version bump may have removed or reassigned streams).
        for s in &tracker.streams {
            self.pid_to_program.remove(&s.pid);
        }
        tracker.streams = stream_infos;
        // Populate pid_to_program for the newly accepted streams.
        for s in &tracker.streams {
            self.pid_to_program.insert(s.pid, program_number);
        }
        tracker.klv_mismatch_coalesce.clear();

        // Emit ProgramMap event.
        self.queue.push_back(DemuxEvent::ProgramMap(prog_map));
        self.program_maps_seen += 1;
        self.pmt_versions_seen += 1;
    }

    /// Build a `ProgramMap` event payload from a parsed PMT and the accepted
    /// stream list (after cross-program collision filtering).
    pub(super) fn build_program_map(
        &mut self,
        pmt: &Pmt,
        program_number: u16,
        streams: &[StreamInfo],
    ) -> ProgramMap {
        let mut klv_pids: Vec<(u16, Option<u16>)> = Vec::new();
        let mut video_pids: Vec<u16> = Vec::new();
        for info in streams {
            if let StreamKind::Video(_) = info.kind {
                video_pids.push(info.pid);
            }
            if matches!(info.kind, StreamKind::KlvSync { .. } | StreamKind::KlvAsync) {
                let declared_link =
                    super::pmt_classify::extract_metadata_link_for_pid(pmt, info.pid);
                klv_pids.push((info.pid, declared_link));
            }
        }
        // Build klv_links table.
        let mut klv_links = Vec::new();
        for (klv_pid, declared) in klv_pids {
            // 1. Caller override wins.
            if let Some(&(_, video_pid)) = self
                .options
                .klv_link_overrides
                .iter()
                .find(|&&(k, _)| k == klv_pid)
            {
                klv_links.push(KlvLink {
                    klv_pid,
                    video_pid,
                    source: LinkSource::Override,
                });
                continue;
            }
            // 2. Declared via metadata_descriptor.
            if let Some(video_pid) = declared {
                klv_links.push(KlvLink {
                    klv_pid,
                    video_pid,
                    source: LinkSource::Declared,
                });
                continue;
            }
            // 3. Inferred — exactly one video PID in this PMT.
            if video_pids.len() == 1 {
                klv_links.push(KlvLink {
                    klv_pid,
                    video_pid: video_pids[0],
                    source: LinkSource::Inferred,
                });
            }
            // 4. Otherwise: no entry. Surface MissingMetadataDescriptor as
            // non-conformant.
            else {
                let stream = StreamId {
                    pid: klv_pid,
                    kind: StreamKind::KlvSync {
                        declared_link: None,
                    },
                    program_number,
                };
                self.queue_nonconformant(stream, NonConformantIssue::MissingMetadataDescriptor);
            }
        }
        ProgramMap {
            program_number,
            pcr_pid: pmt.pcr_pid,
            streams: streams.to_vec(),
            klv_links,
        }
    }

    pub(super) fn klv_mismatch_insert(&mut self, pid: u16) -> bool {
        // Find the tracker that owns this PID via its streams list.
        for tracker in self.programs.values_mut() {
            if tracker.streams.iter().any(|s| s.pid == pid) {
                return tracker.klv_mismatch_coalesce.insert(pid);
            }
        }
        // No tracker found — no suppression.
        true
    }
}
