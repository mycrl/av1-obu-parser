pub mod buffer;
pub mod obu;

/// IVF container parsing helpers for AV1 bitstreams.
///
/// IVF is a simple byte-oriented container:
/// - file header: 32 bytes, beginning with the `DKIF` signature
/// - repeated frame records: 12-byte frame header + frame payload
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IvfHeader {
    pub version: u16,
    pub header_size: u16,
    pub codec: [u8; 4],
    pub width: u16,
    pub height: u16,
    pub timebase_denominator: u32,
    pub timebase_numerator: u32,
    pub num_frames: u32,
    pub unused: u32,
}

impl IvfHeader {
    pub fn codec_string(&self) -> &str {
        std::str::from_utf8(&self.codec).unwrap_or("unknown")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IvfFrame<'a> {
    pub index: usize,
    pub pts: u64,
    pub data: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IvfError {
    InvalidSignature,
    TruncatedHeader,
    InvalidHeaderSize(u16),
    TruncatedFrameHeader {
        frame_index: usize,
    },
    TruncatedFrameData {
        frame_index: usize,
        frame_size: usize,
        remaining_bytes: usize,
    },
}

impl std::fmt::Display for IvfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IvfError::InvalidSignature => {
                write!(f, "file does not start with IVF signature 'DKIF'")
            }
            IvfError::TruncatedHeader => write!(f, "IVF file must be at least 32 bytes"),
            IvfError::InvalidHeaderSize(size) => write!(f, "invalid IVF header size: {size}"),
            IvfError::TruncatedFrameHeader { frame_index } => {
                write!(f, "truncated IVF frame header at frame {frame_index}")
            }
            IvfError::TruncatedFrameData {
                frame_index,
                frame_size,
                remaining_bytes,
            } => write!(
                f,
                "truncated IVF frame data at frame {frame_index}: expected {frame_size} bytes, only {remaining_bytes} remain"
            ),
        }
    }
}

impl std::error::Error for IvfError {}

pub struct IvfReader<'a> {
    data: &'a [u8],
    header: IvfHeader,
}

impl<'a> IvfReader<'a> {
    pub fn new(data: &'a [u8]) -> Result<Self, IvfError> {
        if data.len() < 32 {
            return Err(IvfError::TruncatedHeader);
        }
        if !data.starts_with(b"DKIF") {
            return Err(IvfError::InvalidSignature);
        }

        let header_size = u16::from_le_bytes([data[6], data[7]]);
        if header_size < 32 || header_size as usize > data.len() {
            return Err(IvfError::InvalidHeaderSize(header_size));
        }

        let header = IvfHeader {
            version: u16::from_le_bytes([data[4], data[5]]),
            header_size,
            codec: [data[8], data[9], data[10], data[11]],
            width: u16::from_le_bytes([data[12], data[13]]),
            height: u16::from_le_bytes([data[14], data[15]]),
            timebase_denominator: u32::from_le_bytes([data[16], data[17], data[18], data[19]]),
            timebase_numerator: u32::from_le_bytes([data[20], data[21], data[22], data[23]]),
            num_frames: u32::from_le_bytes([data[24], data[25], data[26], data[27]]),
            unused: u32::from_le_bytes([data[28], data[29], data[30], data[31]]),
        };

        Ok(Self { data, header })
    }

    pub fn header(&self) -> &IvfHeader {
        &self.header
    }

    pub fn frames(&self) -> IvfFrames<'a> {
        IvfFrames {
            data: self.data,
            offset: self.header.header_size as usize,
            frame_index: 0,
            failed: false,
        }
    }
}

pub struct IvfFrames<'a> {
    data: &'a [u8],
    offset: usize,
    frame_index: usize,
    failed: bool,
}

impl<'a> Iterator for IvfFrames<'a> {
    type Item = Result<IvfFrame<'a>, IvfError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.offset >= self.data.len() {
            return None;
        }

        if self.offset + 12 > self.data.len() {
            self.failed = true;
            return Some(Err(IvfError::TruncatedFrameHeader {
                frame_index: self.frame_index,
            }));
        }

        let frame_size = u32::from_le_bytes([
            self.data[self.offset],
            self.data[self.offset + 1],
            self.data[self.offset + 2],
            self.data[self.offset + 3],
        ]) as usize;
        
        let pts = u64::from_le_bytes([
            self.data[self.offset + 4],
            self.data[self.offset + 5],
            self.data[self.offset + 6],
            self.data[self.offset + 7],
            self.data[self.offset + 8],
            self.data[self.offset + 9],
            self.data[self.offset + 10],
            self.data[self.offset + 11],
        ]);

        let data_offset = self.offset + 12;

        if data_offset + frame_size > self.data.len() {
            self.failed = true;
            return Some(Err(IvfError::TruncatedFrameData {
                frame_index: self.frame_index,
                frame_size,
                remaining_bytes: self.data.len().saturating_sub(data_offset),
            }));
        }

        let frame = IvfFrame {
            index: self.frame_index,
            pts,
            data: &self.data[data_offset..data_offset + frame_size],
        };

        self.offset = data_offset + frame_size;
        self.frame_index += 1;
        Some(Ok(frame))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_ivf(frame_payloads: &[&[u8]]) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(b"DKIF");
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&32u16.to_le_bytes());
        data.extend_from_slice(b"AV01");
        data.extend_from_slice(&640u16.to_le_bytes());
        data.extend_from_slice(&480u16.to_le_bytes());
        data.extend_from_slice(&30u32.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&(frame_payloads.len() as u32).to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());

        for (index, payload) in frame_payloads.iter().enumerate() {
            data.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            data.extend_from_slice(&(index as u64).to_le_bytes());
            data.extend_from_slice(payload);
        }

        data
    }

    #[test]
    fn test_parse_ivf_header() {
        let data = build_ivf(&[b"\x12\x00"]);
        let ivf = IvfReader::new(&data).expect("valid IVF header");

        assert_eq!(ivf.header().codec, *b"AV01");
        assert_eq!(ivf.header().codec_string(), "AV01");
        assert_eq!(ivf.header().width, 640);
        assert_eq!(ivf.header().height, 480);
        assert_eq!(ivf.header().num_frames, 1);
    }

    #[test]
    fn test_iterate_ivf_frames() {
        let data = build_ivf(&[b"\x12\x00", b"\x0A\x01\x02"]);
        let ivf = IvfReader::new(&data).expect("valid IVF data");
        let frames = ivf
            .frames()
            .collect::<Result<Vec<_>, _>>()
            .expect("valid IVF frames");

        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].index, 0);
        assert_eq!(frames[0].pts, 0);
        assert_eq!(frames[0].data, b"\x12\x00");
        assert_eq!(frames[1].index, 1);
        assert_eq!(frames[1].pts, 1);
        assert_eq!(frames[1].data, b"\x0A\x01\x02");
    }
}
