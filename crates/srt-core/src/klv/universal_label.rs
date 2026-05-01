//! UniversalLabel — 16-byte SMPTE/MISB key. Filled in Task 3.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UniversalLabel(pub [u8; 16]);

impl std::fmt::Display for UniversalLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}
