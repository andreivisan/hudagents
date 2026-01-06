use std::sync::Arc;

// Used to referebce Blob Object inside the Message Payloa
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct BlobRef(pub u64);

// Used to store Blob objects inside the internal storage
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct BlobId(pub u64);

#[derive(Clone, Debug)]
pub struct Blob {
    pub bytes: Arc<[u8]>,
    pub mime: Option<&'static str>,
}

pub struct BlobStore {}
