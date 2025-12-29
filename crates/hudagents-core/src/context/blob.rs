use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct BlobRef(pub u64);

#[derive(Clone, Debug)]
pub struct Blob {
    pub bytes: Arc<[u8]>,
    pub mime: Option<&'static str>,
}
