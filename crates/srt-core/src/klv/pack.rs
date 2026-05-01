//! Generic KLV pack/unpack. Filled in Tasks 7-9.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawField<'a> {
    pub tag: u32,
    pub value: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedRawField {
    pub tag: u32,
    pub value: Vec<u8>,
}

pub struct Iter<'a> {
    _phantom: std::marker::PhantomData<&'a [u8]>,
}
