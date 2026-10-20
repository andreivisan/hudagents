//! JPEG and WAV messages: a 10-byte header followed by a borrowed media payload.

use crate::device::RequestId;

pub const BINARY_FORMAT_VERSION: u8 = 1;
pub const MEDIA_HEADER_BYTES: usize = 10;
pub const MAX_BINARY_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_WAV_PAYLOAD_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MediaKind {
    Jpeg = 1,
    Wav = 2,
}

impl TryFrom<u8> for MediaKind {
    type Error = MediaFrameError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(MediaKind::Jpeg),
            2 => Ok(MediaKind::Wav),
            _ => Err(MediaFrameError::UnknownKind { received: value }),
        }
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum MediaFrameError {
    #[error("media message is {len} bytes; header requires {MEDIA_HEADER_BYTES} bytes")]
    TooShort { len: usize },

    #[error(
        "media message is {len} bytes; limit is {MAX_BINARY_MESSAGE_BYTES} bytes including the header"
    )]
    MessageTooLarge { len: usize },

    #[error("unsupported binary format version {received}; expected {BINARY_FORMAT_VERSION}")]
    UnsupportedVersion { received: u8 },

    #[error("unknown media kind {received}; expected 1 (JPEG) or 2 (WAV)")]
    UnknownKind { received: u8 },

    #[error("media payload must not be empty")]
    EmptyPayload,

    #[error("media payload is {len} bytes; limit for this media kind is {limit} bytes")]
    PayloadTooLarge { len: usize, limit: usize },
}

/// The payload borrows the input buffer and cannot outlive it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaFrame<'a> {
    pub kind: MediaKind,
    pub request_id: RequestId,
    pub payload: &'a [u8],
}

impl<'a> MediaFrame<'a> {
    /// Validates the envelope without allocating or copying the payload.
    pub fn decode(bytes: &'a [u8]) -> Result<Self, MediaFrameError> {
        if bytes.len() > MAX_BINARY_MESSAGE_BYTES {
            return Err(MediaFrameError::MessageTooLarge { len: bytes.len() });
        }
        if bytes.len() < MEDIA_HEADER_BYTES {
            return Err(MediaFrameError::TooShort { len: bytes.len() });
        }
        if bytes[0] != BINARY_FORMAT_VERSION {
            return Err(MediaFrameError::UnsupportedVersion { received: bytes[0] });
        }
        let media_kind = MediaKind::try_from(bytes[1])?;
        let mut request_id_bytes = [0; 8];
        request_id_bytes.copy_from_slice(&bytes[2..MEDIA_HEADER_BYTES]);
        let request_id = RequestId::new(u64::from_be_bytes(request_id_bytes));
        let payload = &bytes[MEDIA_HEADER_BYTES..];
        validate_payload(media_kind, payload)?;
        Ok(MediaFrame {
            kind: media_kind,
            request_id,
            payload,
        })
    }

    /// Validates the payload before allocating one buffer for the complete message.
    pub fn encode(&self) -> Result<Vec<u8>, MediaFrameError> {
        validate_payload(self.kind, self.payload)?;

        let mut bytes = Vec::with_capacity(MEDIA_HEADER_BYTES + self.payload.len());
        bytes.push(BINARY_FORMAT_VERSION);
        bytes.push(self.kind as u8);
        bytes.extend_from_slice(&self.request_id.get().to_be_bytes());
        bytes.extend_from_slice(self.payload);
        Ok(bytes)
    }
}

fn validate_payload(kind: MediaKind, payload: &[u8]) -> Result<(), MediaFrameError> {
    if payload.is_empty() {
        return Err(MediaFrameError::EmptyPayload);
    }
    let limit = match kind {
        MediaKind::Jpeg => MAX_BINARY_MESSAGE_BYTES - MEDIA_HEADER_BYTES,
        MediaKind::Wav => MAX_WAV_PAYLOAD_BYTES,
    };
    if payload.len() > limit {
        return Err(MediaFrameError::PayloadTooLarge {
            len: payload.len(),
            limit,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Deliberately use literal bytes: the encoder must agree with the wire contract.
    const JPEG: [u8; 12] = [1, 1, 1, 2, 3, 4, 5, 6, 7, 8, 0xAA, 0xBB];
    const WAV: [u8; 12] = [1, 2, 1, 2, 3, 4, 5, 6, 7, 8, 0xAA, 0xBB];
    const REQUEST_ID: u64 = 0x0102030405060708;

    #[test]
    fn constants_match_the_documented_wire_contract() {
        assert_eq!(BINARY_FORMAT_VERSION, 1);
        assert_eq!(MEDIA_HEADER_BYTES, 10);
        assert_eq!(MAX_BINARY_MESSAGE_BYTES, 8_388_608);
        assert_eq!(MAX_WAV_PAYLOAD_BYTES, 4_194_304);
        assert_eq!(std::mem::size_of::<MediaKind>(), 1);
    }

    #[test]
    fn encoding_matches_literal_jpeg_and_wav_messages() {
        for (kind, expected) in [(MediaKind::Jpeg, JPEG), (MediaKind::Wav, WAV)] {
            let frame = MediaFrame {
                kind,
                request_id: RequestId::new(REQUEST_ID),
                payload: &[0xAA, 0xBB],
            };
            assert_eq!(frame.encode().unwrap(), expected);
        }
    }

    #[test]
    fn decoding_literal_messages_reads_big_endian_id_and_borrows_payload() {
        for (kind, bytes) in [(MediaKind::Jpeg, JPEG), (MediaKind::Wav, WAV)] {
            let decoded = MediaFrame::decode(&bytes).unwrap();
            assert_eq!(decoded.kind, kind);
            assert_eq!(decoded.request_id.get(), REQUEST_ID);
            assert_eq!(decoded.payload, &[0xAA, 0xBB]);
            assert_eq!(decoded.payload.as_ptr(), bytes[10..].as_ptr());
        }
    }

    #[test]
    fn round_trips_both_kinds_and_request_id_boundaries_without_copying_payload() {
        // Payload contents are opaque to this codec, including zero and non-UTF-8 bytes.
        let payload = [0x00, 0xFF, 0x80, 0x01, 0x00];
        for kind in [MediaKind::Jpeg, MediaKind::Wav] {
            for id in [0, 1, 42, (1_u64 << 53) + 1, u64::MAX] {
                let frame = MediaFrame {
                    kind,
                    request_id: RequestId::new(id),
                    payload: &payload,
                };
                let encoded = frame.encode().unwrap();
                let decoded = MediaFrame::decode(&encoded).unwrap();
                assert_eq!(decoded, frame);
                assert_eq!(
                    decoded.payload.as_ptr(),
                    encoded[MEDIA_HEADER_BYTES..].as_ptr()
                );
            }
        }
    }

    #[test]
    fn kind_conversion_and_decoding_reject_every_unassigned_kind_byte() {
        for value in u8::MIN..=u8::MAX {
            let expected = match value {
                1 => Ok(MediaKind::Jpeg),
                2 => Ok(MediaKind::Wav),
                _ => Err(MediaFrameError::UnknownKind { received: value }),
            };
            assert_eq!(MediaKind::try_from(value), expected);
            let mut bytes = JPEG;
            bytes[1] = value;
            assert_eq!(MediaFrame::decode(&bytes).map(|frame| frame.kind), expected);
        }
    }

    #[test]
    fn rejects_every_truncated_header_length() {
        for len in 0..MEDIA_HEADER_BYTES {
            assert_eq!(
                MediaFrame::decode(&JPEG[..len]),
                Err(MediaFrameError::TooShort { len })
            );
        }
    }

    #[test]
    fn rejects_every_unsupported_version_byte() {
        for received in u8::MIN..=u8::MAX {
            let mut bytes = JPEG;
            bytes[0] = received;
            if received == BINARY_FORMAT_VERSION {
                assert!(MediaFrame::decode(&bytes).is_ok());
            } else {
                assert_eq!(
                    MediaFrame::decode(&bytes),
                    Err(MediaFrameError::UnsupportedVersion { received })
                );
            }
        }
    }

    #[test]
    fn header_only_messages_and_empty_payload_encoding_are_rejected() {
        for (kind, bytes) in [(MediaKind::Jpeg, JPEG), (MediaKind::Wav, WAV)] {
            assert_eq!(
                MediaFrame::decode(&bytes[..MEDIA_HEADER_BYTES]),
                Err(MediaFrameError::EmptyPayload)
            );
            let frame = MediaFrame {
                kind,
                request_id: RequestId::new(42),
                payload: &[],
            };
            assert_eq!(frame.encode(), Err(MediaFrameError::EmptyPayload));
        }
    }

    #[test]
    fn accepts_one_byte_payloads() {
        for (kind, bytes) in [(MediaKind::Jpeg, JPEG), (MediaKind::Wav, WAV)] {
            let decoded = MediaFrame::decode(&bytes[..11]).unwrap();
            assert_eq!(decoded.kind, kind);
            assert_eq!(decoded.payload, &[0xAA]);
            assert_eq!(decoded.encode().unwrap(), bytes[..11]);
        }
    }

    #[test]
    fn oversized_message_is_rejected_before_header_parsing() {
        let len = MAX_BINARY_MESSAGE_BYTES + 1;
        // Both version and kind are invalid, but the size error must take precedence.
        let bytes = vec![0xFF; len];
        assert_eq!(
            MediaFrame::decode(&bytes),
            Err(MediaFrameError::MessageTooLarge { len })
        );
    }

    #[test]
    fn header_validation_precedes_payload_validation() {
        let mut bytes = [0; MEDIA_HEADER_BYTES];
        bytes[0] = 2;
        bytes[1] = 99;
        assert_eq!(
            MediaFrame::decode(&bytes),
            Err(MediaFrameError::UnsupportedVersion { received: 2 })
        );
        bytes[0] = BINARY_FORMAT_VERSION;
        assert_eq!(
            MediaFrame::decode(&bytes),
            Err(MediaFrameError::UnknownKind { received: 99 })
        );
        bytes[1] = 1;
        assert_eq!(
            MediaFrame::decode(&bytes),
            Err(MediaFrameError::EmptyPayload)
        );
    }

    #[test]
    fn jpeg_limit_includes_header_and_rejects_one_extra_byte() {
        let limit = MAX_BINARY_MESSAGE_BYTES - MEDIA_HEADER_BYTES;
        let payload = vec![0xAA; limit + 1];
        let frame = MediaFrame {
            kind: MediaKind::Jpeg,
            request_id: RequestId::new(42),
            payload: &payload[..limit],
        };
        let mut encoded = frame.encode().unwrap();
        assert_eq!(encoded.len(), MAX_BINARY_MESSAGE_BYTES);
        assert_eq!(MediaFrame::decode(&encoded).unwrap(), frame);

        let oversized_frame = MediaFrame {
            payload: &payload,
            ..frame
        };
        assert_eq!(
            oversized_frame.encode(),
            Err(MediaFrameError::PayloadTooLarge {
                len: limit + 1,
                limit,
            })
        );

        encoded.push(0xAA);
        assert_eq!(
            MediaFrame::decode(&encoded),
            Err(MediaFrameError::MessageTooLarge {
                len: MAX_BINARY_MESSAGE_BYTES + 1,
            })
        );
    }

    #[test]
    fn wav_limit_excludes_media_header_and_rejects_one_extra_byte() {
        let limit = MAX_WAV_PAYLOAD_BYTES;
        let payload = vec![0xAA; limit + 1];
        let frame = MediaFrame {
            kind: MediaKind::Wav,
            request_id: RequestId::new(42),
            payload: &payload[..limit],
        };
        let mut encoded = frame.encode().unwrap();
        assert_eq!(encoded.len(), MEDIA_HEADER_BYTES + limit);
        assert_eq!(MediaFrame::decode(&encoded).unwrap(), frame);

        let oversized_frame = MediaFrame {
            payload: &payload,
            ..frame
        };
        assert_eq!(
            oversized_frame.encode(),
            Err(MediaFrameError::PayloadTooLarge {
                len: limit + 1,
                limit,
            })
        );

        encoded.push(0xAA);
        assert_eq!(
            MediaFrame::decode(&encoded),
            Err(MediaFrameError::PayloadTooLarge {
                len: limit + 1,
                limit,
            })
        );
    }

    #[test]
    fn jpeg_can_exceed_the_wav_payload_limit() {
        let payload = vec![0xAA; MAX_WAV_PAYLOAD_BYTES + 1];
        let frame = MediaFrame {
            kind: MediaKind::Jpeg,
            request_id: RequestId::new(42),
            payload: &payload,
        };
        let encoded = frame.encode().unwrap();
        assert_eq!(MediaFrame::decode(&encoded).unwrap(), frame);
    }

    #[test]
    fn errors_describe_the_rejected_value_or_limit() {
        let examples = [
            (
                MediaFrameError::TooShort { len: 9 },
                "media message is 9 bytes; header requires 10 bytes",
            ),
            (
                MediaFrameError::MessageTooLarge { len: 8_388_609 },
                "media message is 8388609 bytes; limit is 8388608 bytes including the header",
            ),
            (
                MediaFrameError::UnsupportedVersion { received: 2 },
                "unsupported binary format version 2; expected 1",
            ),
            (
                MediaFrameError::UnknownKind { received: 99 },
                "unknown media kind 99; expected 1 (JPEG) or 2 (WAV)",
            ),
            (
                MediaFrameError::EmptyPayload,
                "media payload must not be empty",
            ),
            (
                MediaFrameError::PayloadTooLarge {
                    len: 4_194_305,
                    limit: 4_194_304,
                },
                "media payload is 4194305 bytes; limit for this media kind is 4194304 bytes",
            ),
        ];
        for (error, expected) in examples {
            assert_eq!(error.to_string(), expected);
        }
    }
}
