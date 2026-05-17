# Workspace Conventions

This file codifies workspace-wide policies that span crates. Per-policy
rationale, with examples, follows each rule. New contributors check this
file before introducing new public types or constructors; codex reviewers
verify new code against these rules.

For audit-trace context, this doc was created in plan #72 (Wave 2.3,
2026-05-18) to consolidate the recommendations from
`docs/refactor-1/02-naming-consistency.md` Findings 3, 4, 5 and
`docs/refactor-1/03-architecture.md` Finding 6.

---

## Config vs Options: use `Config` workspace-wide

**Rule:** Public types holding caller-supplied construction parameters
are named `<Component>Config`. Do not use the `Options` suffix.

**Why:** Two suffixes for the same concept (`MuxerConfig` vs
`DemuxerOptions`) force readers to guess which suffix a particular type
uses. The conventions audit found `Config` already dominates
(`MuxerConfig`, `MuxerProgramConfig`, `SenderConfig`, `RawSenderConfig`,
`SocketConfig`, `ListenerConfig`); the outliers (`DemuxerOptions`,
`PairerOptions`, `EncodeOptions`) are migrated to `Config` in this
plan.

**Examples:**

```rust
// Good — workspace convention:
pub struct SenderConfig { ... }
pub struct RawSenderConfig { ... }
pub struct ReceiverConfig { ... }
pub struct PairerConfig { ... }
pub struct EncodeConfig { ... }

// Bad — do not use:
pub struct PairerOptions { ... }  // Was renamed PairerConfig in plan #72
```

**Note on naming collisions:** `MuxError` has both `InvalidConfig`
(flat-string, pre-existing) and `ConfigInvalid` (new richer variant
from plan #72). The two coexist; the latter is for diagnostics that
need a formatted reason. New error variants should also prefer the
`<Subject><Adjective>` ordering used by the rest of the enum
(`KlvTooLarge`, `BufferFull`, `AudioTooLarge`).

---

## Constructor naming convention

**Rule:** Public constructors on workspace types follow these patterns:

| Pattern | Meaning |
|---------|---------|
| `T::new(...)` | **Primary constructor.** Takes all required arguments. Use this for the canonical way to construct. |
| `T::from_<format>(...)` | **Parsing constructor.** Decodes from a wire format / encoding (`from_bytes`, `from_str_strict`, `from_u8`, `from_h273`, `from_raw`, `from_pts`, `from_millis`, `from_env`, `from_file`). Always reads from input. |
| `T::with_<aspect>(...)` | **Variant constructor.** Takes one optional behavioral knob in addition to the required args. Useful when the knob is part of construction (not chainable later). Example: `Demuxer::with_options(options)`. |
| `T::default()` | **Zero-arg fallback.** Only when `Default` is implementable and meaningful. |
| `T::builder(...)` | **Builder factory.** Returns a builder type that produces `T`. Use the builder pattern (see "Builder vs Default" below) when the rule applies. |

**Why:** Mixed patterns across the workspace (`Pairer::with_options(opts)`,
`Sender::new(transport, config)`, `H264Sps::from_bytes(...)`,
`MuxerConfig::builder()`) confuse callers reading imports. Codifying
the rule lets future code be reviewed against it.

**Examples:**

```rust
// Good:
let sender = Sender::new(transport, config);             // primary
let pairer = Pairer::new(video_pid, klv_pid);            // primary with required args
let sps = H264Sps::from_bytes(nal_payload)?;             // parsing
let demuxer = Demuxer::with_options(options);            // variant
let cfg = MuxerConfig::default();                        // zero-arg fallback
let bldr = MuxerConfig::builder();                       // builder factory

// Acceptable but exceptional — document why if you use them:
let pairer = Pairer::with_options(v, k, opts);           // explicit knob still fine
```

---

## Builder vs Default

**Rule:** Use a chainable `&mut self -> &mut Self` builder when:

- The config has **≥ 2 knobs**, OR
- The config requires **validation** that should run at build time, OR
- Construction has **required fields** that are best collected
  positionally (`Builder::new(required_arg)` then chainable setters).

Use `#[derive(Default)]` plain struct (or a manual `Default` impl) when:

- The config is **empty / single-knob**, AND
- **No validation logic** exists, AND
- All fields are **independently safe** to set in any order.

**Why:** No documented rule means new contributors invent local
conventions. The rule above matches the workspace's existing shape:
`MuxerConfigBuilder` + `MuxerProgramConfigBuilder` + `SocketBuilder` +
`ListenerBuilder` use chainable `&mut self -> &mut Self` (the Phase 3
shape). `RawSenderConfig` is empty/`#[derive(Default)]` with no builder.

**Examples:**

```rust
// Builder warranted (≥2 knobs + validation):
pub struct SocketBuilder { ... }
impl SocketBuilder {
    pub fn new() -> Self { ... }
    pub fn max_payload(&mut self, n: usize) -> &mut Self { ... }
    pub fn passphrase(&mut self, p: Passphrase) -> &mut Self { ... }
    pub fn connect(&self, addr: ...) -> Result<Socket, ...> { ... }  // build + validate
}

// Plain struct sufficient (empty config, no validation):
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct RawSenderConfig {}
```

---

## Public field policy for `*Config` structs

**Rule:** Public `*Config` structs in this workspace use
**field-public + `#[non_exhaustive]`**. New fields can be added
non-breakingly; struct-literal construction is denied outside the
crate (per Rust E0639 for `#[non_exhaustive]`), so callers must use
`Config::default()` and assign overrides, or use the corresponding
builder where one exists.

**Why:** Decided in the Wave 2 brainstorming session
(`docs/refactor-1/_wave-2-plan-design.md`):

- **Considered alternative:** opaque builder-owned configs with all
  fields private. Rejected because the migration cost across ~20
  workspace config types is substantial, and the user-confirmed
  "break-freely pre-1.0" policy (`feedback_break_freely_prerelease.md`)
  means we can revisit this post-1.0 if the field-public shape proves
  problematic. For now, field-public + `#[non_exhaustive]` gives 95%
  of the future-proofing benefit at 10% of the implementation cost.
- **Builder coexistence:** types that have a `*ConfigBuilder` (like
  `MuxerConfig`) keep both. The builder remains the recommended
  ergonomic path; the field-public form is the FFI-friendly path for
  bindings that don't want to thread a builder.

**Construction patterns callers should use:**

```rust
// Recommended for Rust callers — default-and-assign:
let mut cfg = SocketConfig::default();
cfg.payload_size = Some(1316);
cfg.passphrase = Some(Passphrase::from_env("MY_KEY")?);

// For MuxerProgramConfig (no Default impl), use the in-crate constructor:
let mut prog = MuxerProgramConfig::new(1, 0x1000);
prog.streams = vec![StreamSpec::Video { pid: 0x1011, codec: VideoCodec::H264 }];
prog.stream_descriptors = vec![vec![]];

// Or via builder where available:
let mut bldr = MuxerConfig::builder();
bldr.add_program(MuxerProgramConfigBuilder::new(1, 0x1000).build());
let cfg = bldr.build()?;

// NOT supported (Rust compile error E0639):
let cfg = SocketConfig { payload_size: Some(1316), ..Default::default() };
```

---

## Where construction invariants are enforced

**Rule:** Validation of cross-field invariants on `*Config` structs
happens at the **constructor boundary** that consumes the config —
`Muxer::new(config)`, `Sender::new(transport, config)`,
`Socket::open(config)`, etc. — by calling `config.validate()` (or
equivalent) and returning `Err(...)` on rejection.

**Why:** Public-field configs (per the rule above) can be hand-built
with struct-update syntax (`SomeConfig { field: x, ..Default::default() }`
where `#[non_exhaustive]` permits) or via builders. Either way, the
constructor that ultimately consumes the config is the choke-point
where invariants are guaranteed to be checked. Catching invalid
configs at construction-time is preferable to panicking later from
inside the muxer or transport.

**Example — `MuxerProgramConfig.stream_descriptors` length invariant:**

Pre-plan-#72, the check at `MuxerConfig::validate()` raised
`MuxError::InvalidConfig("stream_descriptors.len() must equal streams.len()")` —
a flat static string with no diagnostic context.

Post-plan-#72, the check raises `MuxError::ConfigInvalid { reason }`
with a formatted reason naming the program number, actual streams
count, and actual stream_descriptors count. The richer diagnostic
helps callers locate the offending program in a multi-program config
without re-reading the code.

```rust
impl MuxerConfig {
    pub fn validate(&self) -> Result<(), MuxError> {
        for prog in &self.programs {
            if prog.stream_descriptors.len() != prog.streams.len() {
                return Err(MuxError::ConfigInvalid {
                    reason: format!(
                        "program {} has {} streams but {} stream_descriptors \
                         (lengths must match)",
                        prog.program_number,
                        prog.streams.len(),
                        prog.stream_descriptors.len(),
                    ),
                });
            }
            // ... other checks ...
        }
        Ok(())
    }
}
```

The constructor-boundary approach is preferred over method-boundary
checks (catching invariants only when `push_video` is called, for
example) because the failure surfaces at construction-time, where the
caller still has the config in hand to fix.

---

## See also

- `docs/binding-authors.md` § "Builder ownership patterns" — distinguishes
  reusable builders (`SocketBuilder::connect(&self)`) from consuming
  constructors (`Sender::new(transport, config)`).
- `docs/architecture.md` — crate boundaries and ownership.
- `feedback_break_freely_prerelease.md` (memory) — pre-1.0 break policy.
