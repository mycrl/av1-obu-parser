/// AV1 bitstream reader.
///
/// AV1 bitstreams are read MSB-first (high bit first). This struct holds a
/// borrowed byte slice and tracks the current byte index and bit offset within
/// the current byte.
pub struct Buffer<'a> {
    buf: &'a [u8],
    /// Current byte index into `buf`.
    index: usize,
    /// Bit offset within the current byte (0 = MSB).
    bit_pos: usize,
}

impl<'a> Buffer<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self {
            buf,
            index: 0,
            bit_pos: 0,
        }
    }

    /// Skip `n` bits without returning a value.
    pub fn seek_bits(&mut self, cut: usize) {
        for _ in 0..cut {
            self.advance();
        }
    }

    /// Read `count` bytes as a slice. Requires byte alignment.
    pub fn get_bytes(&mut self, count: usize) -> &[u8] {
        assert_eq!(self.bit_pos, 0, "get_bytes requires byte alignment");
        self.index += count;
        &self.buf[self.index - count..self.index]
    }

    /// Read 1 bit and return it as a bool.
    pub fn get_bit(&mut self) -> bool {
        self.next()
    }

    /// f(n): read `count` bits MSB-first as an unsigned integer.
    ///
    /// AV1 spec Section 4.10.2 - f(n).
    pub fn get_bits(&mut self, count: usize) -> u32 {
        assert!(count > 0 && count <= 32, "count must be in [1, 32]");

        let mut aac = 0;
        for i in 0..count {
            aac |= (self.get_bit() as u32) << (count - i - 1);
        }
        aac
    }

    /// uvlc(): variable-length unsigned integer.
    ///
    /// AV1 spec Section 4.10.3 - uvlc().
    /// Counts leading zeros `lz`, then reads `lz` additional bits.
    pub fn get_uvlc(&mut self) -> u32 {
        let mut lz = 0;
        loop {
            if self.get_bit() {
                break;
            }
            lz += 1;
        }

        if lz >= 32 {
            0xFFFFFFFF
        } else {
            self.get_bits(lz) + (1 << lz) - 1
        }
    }

    /// le(n): unsigned little-endian n-byte integer. Requires byte alignment.
    ///
    /// AV1 spec Section 4.10.4 - le(n).
    pub fn get_le(&mut self, count: usize) -> u32 {
        assert_eq!(self.bit_pos, 0, "get_le requires byte alignment");

        let mut t = 0;
        for i in 0..count {
            t += self.get_bits(8) << (i * 8);
        }
        t
    }

    /// leb128(): variable-length LEB128 unsigned integer. Requires byte alignment.
    ///
    /// AV1 spec Section 4.10.5 - leb128().
    /// The MSB of each byte is a continuation flag (1 = more bytes follow).
    /// The lower 7 bits of each byte carry the value.
    /// At most 8 bytes are read; the spec requires the result to fit in 32 bits.
    pub fn get_leb128(&mut self) -> u64 {
        assert_eq!(self.bit_pos, 0, "get_leb128 requires byte alignment");

        let mut value: u64 = 0;
        for i in 0..8u64 {
            let byte = self.get_bits(8) as u64;
            value |= (byte & 0x7f) << (i * 7);
            if byte & 0x80 == 0 {
                break;
            }
        }
        value
    }

    /// su(n): n-bit signed integer.
    ///
    /// AV1 spec Section 4.10.6 - su(n).
    /// Reads an n-bit unsigned value and sign-extends via the MSB.
    pub fn get_su(&mut self, count: usize) -> i32 {
        let value = self.get_bits(count) as i32;
        let sign_mask = 1i32 << (count - 1);
        if value & sign_mask != 0 {
            value - 2 * sign_mask
        } else {
            value
        }
    }

    /// ns(n): non-symmetric unsigned coded integer in the range [0, n-1].
    ///
    /// AV1 spec Section 4.10.7 - ns(n).
    /// Values below a threshold are coded with `floor(log2(n))` bits;
    /// values at or above the threshold use one extra bit, saving bits overall.
    pub fn get_ns(&mut self, n: u32) -> u32 {
        if n <= 1 {
            return 0;
        }
        // w = floor_log2(n) + 1 = number of bits needed to represent n
        let w = (32 - n.leading_zeros()) as usize;
        // The first m symbols need only w-1 bits
        let m = (1u32 << w) - n;
        let v = self.get_bits(w - 1);
        if v < m {
            v
        } else {
            let extra_bit = self.get_bit() as u32;
            (v << 1) - m + extra_bit
        }
    }

    /// Returns `true` if the current position is byte-aligned.
    pub fn is_byte_aligned(&self) -> bool {
        self.bit_pos == 0
    }

    /// Advance to the next byte boundary, discarding any remaining bits in the
    /// current byte (trailing_bits padding).
    pub fn byte_align(&mut self) {
        if self.bit_pos != 0 {
            self.seek_bits(8 - self.bit_pos);
        }
    }

    /// Returns the number of whole bytes remaining (rounded down).
    pub fn bytes_remaining(&self) -> usize {
        if self.index >= self.buf.len() {
            return 0;
        }
        self.buf.len() - self.index
    }

    /// Returns the number of bytes consumed so far (rounded up to the nearest byte).
    pub fn bytes_consumed(&self) -> usize {
        self.index + if self.bit_pos > 0 { 1 } else { 0 }
    }
}

impl<'a> Buffer<'a> {
    /// Advance the internal position by one bit.
    fn advance(&mut self) {
        self.bit_pos += 1;
        if self.bit_pos == 8 {
            self.bit_pos = 0;
            if self.index < self.buf.len() {
                self.index += 1;
            }
        }
    }

    /// Read the current bit and advance.
    fn next(&mut self) -> bool {
        let curr_byte = self.buf[self.index];
        let shift = 7 - self.bit_pos;
        let bit = curr_byte & (1 << shift);
        self.advance();
        (bit >> shift) == 1
    }
}

impl<'a> AsMut<Buffer<'a>> for Buffer<'a> {
    fn as_mut(&mut self) -> &mut Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_bit() {
        // 0b10110010 = 0xB2
        let data = [0xB2u8];
        let mut buf = Buffer::new(&data);
        assert_eq!(buf.get_bit(), true);  // bit 7 = 1
        assert_eq!(buf.get_bit(), false); // bit 6 = 0
        assert_eq!(buf.get_bit(), true);  // bit 5 = 1
        assert_eq!(buf.get_bit(), true);  // bit 4 = 1
        assert_eq!(buf.get_bit(), false); // bit 3 = 0
        assert_eq!(buf.get_bit(), false); // bit 2 = 0
        assert_eq!(buf.get_bit(), true);  // bit 1 = 1
        assert_eq!(buf.get_bit(), false); // bit 0 = 0
    }

    #[test]
    fn test_get_bits() {
        let data = [0xABu8, 0xCDu8]; // 10101011 11001101
        let mut buf = Buffer::new(&data);
        assert_eq!(buf.get_bits(4), 0xA);  // 1010
        assert_eq!(buf.get_bits(4), 0xB);  // 1011
        assert_eq!(buf.get_bits(8), 0xCD); // 11001101
    }

    #[test]
    fn test_get_leb128() {
        // Single-byte LEB128: 5
        let data = [0x05u8];
        let mut buf = Buffer::new(&data);
        assert_eq!(buf.get_leb128(), 5);

        // Two-byte LEB128: 128 encoded as [0x80, 0x01]
        let data2 = [0x80u8, 0x01u8];
        let mut buf2 = Buffer::new(&data2);
        assert_eq!(buf2.get_leb128(), 128);
    }

    #[test]
    fn test_get_su() {
        // su(4): read 1100 = 12; sign bit set, so result = 12 - 16 = -4
        let data = [0b1100_0000u8];
        let mut buf = Buffer::new(&data);
        assert_eq!(buf.get_su(4), -4);
    }

    #[test]
    fn test_get_ns() {
        // ns(4): n=4, w=3, m=(1<<3)-4=4.
        // m=4 means all 2-bit values (0–3) are smaller than m and are returned
        // directly without reading an extra bit.
        let data = [0b00_01_10_11u8];
        let mut buf = Buffer::new(&data);
        assert_eq!(buf.get_ns(4), 0); // 00 → 0
        assert_eq!(buf.get_ns(4), 1); // 01 → 1
        assert_eq!(buf.get_ns(4), 2); // 10 → 2
        assert_eq!(buf.get_ns(4), 3); // 11 → 3 (still < m=4, no extra bit)
    }

    #[test]
    fn test_byte_align() {
        let data = [0xFFu8, 0xAAu8];
        let mut buf = Buffer::new(&data);
        buf.get_bits(3);
        assert!(!buf.is_byte_aligned());
        buf.byte_align();
        assert!(buf.is_byte_aligned());
        assert_eq!(buf.get_bits(8), 0xAA);
    }
}
