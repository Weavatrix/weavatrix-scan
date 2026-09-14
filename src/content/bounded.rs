use crate::control::CancellationToken;
use std::io::{self, Read};
use std::time::Instant;

/// Caps for one file read. Physical I/O may exceed selected content by one
/// sentinel byte used only to detect growth.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BoundedReadLimits {
    pub expected_bytes: u64,
    pub max_content_bytes: u64,
}

/// Cooperative checks evaluated between chunks, never inside a blocked `read`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ReadControl<'a> {
    pub cancellation: Option<&'a CancellationToken>,
    pub deadline: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoundedReadStatus {
    Complete,
    Stopped,
    Grown,
    Cancelled,
    Deadline,
}

pub(crate) fn max_io_bytes(limits: BoundedReadLimits) -> u64 {
    limits
        .expected_bytes
        .min(limits.max_content_bytes)
        .saturating_add(1)
}

/// Reads a file with an expected-size cap, a selected-content cap, and at most
/// one extra physical byte that is never delivered to `on_chunk`.
pub(crate) fn read_bounded<R, F>(
    reader: &mut R,
    buffer: &mut [u8],
    limits: BoundedReadLimits,
    control: ReadControl<'_>,
    mut on_chunk: F,
) -> io::Result<(u64, BoundedReadStatus)>
where
    R: Read,
    F: FnMut(&[u8]) -> io::Result<bool>,
{
    let io_limit = max_io_bytes(limits);
    let content_limit = limits.expected_bytes.min(limits.max_content_bytes);
    let mut bytes_read = 0_u64;
    loop {
        if let Some(status) = interrupted(&control) {
            return Ok((bytes_read, status));
        }
        if bytes_read >= io_limit {
            return Ok((bytes_read, BoundedReadStatus::Grown));
        }
        let remaining = usize::try_from(io_limit.saturating_sub(bytes_read))
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        if remaining == 0 {
            return Ok((bytes_read, BoundedReadStatus::Grown));
        }
        let read = reader.read(&mut buffer[..remaining])?;
        if read == 0 {
            return Ok((bytes_read, BoundedReadStatus::Complete));
        }
        let read_u64 = read as u64;
        let content_room = content_limit.saturating_sub(bytes_read);
        let emit = usize::try_from(content_room).unwrap_or(0).min(read);
        if emit > 0 && !on_chunk(&buffer[..emit])? {
            return Ok((
                bytes_read.saturating_add(emit as u64),
                BoundedReadStatus::Stopped,
            ));
        }
        bytes_read = bytes_read.saturating_add(read_u64);
        if bytes_read > content_limit {
            return Ok((content_limit, BoundedReadStatus::Grown));
        }
    }
}

fn interrupted(control: &ReadControl<'_>) -> Option<BoundedReadStatus> {
    if control
        .cancellation
        .is_some_and(CancellationToken::is_cancelled)
    {
        return Some(BoundedReadStatus::Cancelled);
    }
    if control
        .deadline
        .is_some_and(|deadline| Instant::now() >= deadline)
    {
        return Some(BoundedReadStatus::Deadline);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{BoundedReadLimits, BoundedReadStatus, ReadControl, read_bounded};
    use crate::CancellationToken;
    use std::io::{self, Cursor, Read};
    use std::time::Instant;

    struct ScriptedReader {
        chunks: Vec<Vec<u8>>,
        index: usize,
    }

    impl Read for ScriptedReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let Some(chunk) = self.chunks.get(self.index) else {
                return Ok(0);
            };
            self.index += 1;
            let copied = chunk.len().min(buf.len());
            buf[..copied].copy_from_slice(&chunk[..copied]);
            Ok(copied)
        }
    }

    fn limits(expected: u64, max_content: u64) -> BoundedReadLimits {
        BoundedReadLimits {
            expected_bytes: expected,
            max_content_bytes: max_content,
        }
    }

    #[test]
    fn growth_sentinel_is_not_delivered() {
        let mut reader = Cursor::new(vec![1, 2, 3, 4]);
        let mut buffer = [0_u8; 8];
        let mut seen = Vec::new();
        let (bytes, status) = read_bounded(
            &mut reader,
            &mut buffer,
            limits(2, 8),
            ReadControl::default(),
            |chunk| {
                seen.extend_from_slice(chunk);
                Ok(true)
            },
        )
        .unwrap();
        assert_eq!(status, BoundedReadStatus::Grown);
        assert_eq!(bytes, 2);
        assert_eq!(seen, [1, 2]);
    }

    #[test]
    fn truncation_stops_at_eof() {
        let mut reader = ScriptedReader {
            chunks: vec![vec![9, 8]],
            index: 0,
        };
        let mut buffer = [0_u8; 8];
        let mut seen = Vec::new();
        let (bytes, status) = read_bounded(
            &mut reader,
            &mut buffer,
            limits(8, 8),
            ReadControl::default(),
            |chunk| {
                seen.extend_from_slice(chunk);
                Ok(true)
            },
        )
        .unwrap();
        assert_eq!(status, BoundedReadStatus::Complete);
        assert_eq!(bytes, 2);
        assert_eq!(seen, [9, 8]);
    }

    #[test]
    fn cancel_and_deadline_are_checked_between_chunks() {
        let token = CancellationToken::new();
        token.cancel();
        let mut reader = Cursor::new(vec![1, 2, 3]);
        let mut buffer = [0_u8; 1];
        let (_, status) = read_bounded(
            &mut reader,
            &mut buffer,
            limits(3, 3),
            ReadControl {
                cancellation: Some(&token),
                deadline: None,
            },
            |_| Ok(true),
        )
        .unwrap();
        assert_eq!(status, BoundedReadStatus::Cancelled);

        let mut reader = Cursor::new(vec![1, 2, 3]);
        let (_, status) = read_bounded(
            &mut reader,
            &mut buffer,
            limits(3, 3),
            ReadControl {
                cancellation: None,
                deadline: Some(Instant::now()),
            },
            |_| Ok(true),
        )
        .unwrap();
        assert_eq!(status, BoundedReadStatus::Deadline);
    }

    #[test]
    fn missing_eof_is_treated_as_growth_once_io_cap_is_hit() {
        let mut reader = ScriptedReader {
            chunks: vec![vec![1], vec![2], vec![3], vec![4]],
            index: 0,
        };
        let mut buffer = [0_u8; 1];
        let mut seen = Vec::new();
        let (bytes, status) = read_bounded(
            &mut reader,
            &mut buffer,
            limits(2, 2),
            ReadControl::default(),
            |chunk| {
                seen.extend_from_slice(chunk);
                Ok(true)
            },
        )
        .unwrap();
        assert_eq!(status, BoundedReadStatus::Grown);
        assert_eq!(bytes, 2);
        assert_eq!(seen, [1, 2]);
    }
}
