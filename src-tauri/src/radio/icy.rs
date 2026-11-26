//! Strip interleaved Shoutcast metadata before the native decoder sees bytes.
use crate::library::text::clean;
use std::io;

const MAX_INTERVAL: usize = 1024 * 1024;

pub(super) struct Icy {
    interval: usize,
    audio_left: usize,
    metadata_left: usize,
    metadata: Vec<u8>,
}

impl Icy {
    pub(super) fn new(interval: &str) -> io::Result<Self> {
        let interval = interval
            .parse::<usize>()
            .ok()
            .filter(|value| (1..=MAX_INTERVAL).contains(value))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid ICY interval"))?;
        Ok(Self {
            interval,
            audio_left: interval,
            metadata_left: 0,
            metadata: Vec::new(),
        })
    }

    /// Each length byte announces at most 4,080 bytes, split at arbitrary
    /// response boundaries. Only complete metadata may update the title.
    pub(super) fn strip(&mut self, mut chunk: &[u8]) -> (Vec<u8>, Option<String>) {
        let mut audio = Vec::with_capacity(chunk.len());
        let mut title = None;
        while !chunk.is_empty() {
            if self.audio_left > 0 {
                let count = self.audio_left.min(chunk.len());
                audio.extend_from_slice(&chunk[..count]);
                chunk = &chunk[count..];
                self.audio_left -= count;
            } else if self.metadata_left > 0 {
                let count = self.metadata_left.min(chunk.len());
                self.metadata.extend_from_slice(&chunk[..count]);
                chunk = &chunk[count..];
                self.metadata_left -= count;
                if self.metadata_left == 0 {
                    if let Some(found) = read_title(&self.metadata) {
                        title = Some(found);
                    }
                    self.metadata.clear();
                    self.audio_left = self.interval;
                }
            } else {
                self.metadata_left = usize::from(chunk[0]) * 16;
                chunk = &chunk[1..];
                if self.metadata_left == 0 {
                    self.audio_left = self.interval;
                }
            }
        }
        (audio, title)
    }
}

fn read_title(bytes: &[u8]) -> Option<String> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let text = std::str::from_utf8(&bytes[..end]).ok()?;
    let raw = text.strip_prefix("StreamTitle='")?;
    let (title, _) = raw.split_once("';")?;
    Some(clean(title))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_audio_length_and_metadata_at_every_chunk_boundary() {
        let mut stream = b"abc\x02StreamTitle='Artist - Track';".to_vec();
        stream.resize(3 + 1 + 32, 0);
        stream.extend_from_slice(b"def\x00ghi");
        for split in 0..stream.len() {
            let mut parser = Icy::new("3").unwrap();
            let (mut audio, first) = parser.strip(&stream[..split]);
            let (rest, second) = parser.strip(&stream[split..]);
            audio.extend(rest);
            assert_eq!(audio, b"abcdefghi");
            assert_eq!(second.or(first).as_deref(), Some("Artist - Track"));
        }
    }

    #[test]
    fn malformed_metadata_does_not_leak_into_the_audio() {
        let mut parser = Icy::new("1").unwrap();
        let mut bytes = b"a\x01".to_vec();
        bytes.extend_from_slice(&[255; 16]);
        bytes.extend_from_slice(b"b");
        assert_eq!(parser.strip(&bytes), (b"ab".to_vec(), None));
        assert!(Icy::new("0").is_err());
        assert!(Icy::new("99999999").is_err());
    }
}
