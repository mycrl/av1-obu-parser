/// AV1 bitstream reader.
///
/// This type implements the primitive bit-level operations used by the AV1
/// syntax functions in spec Section 4.10:
///
/// - `f(n)`: fixed-width unsigned bits
/// - `uvlc()`: unsigned Exp-Golomb-like code used by AV1
/// - `le(n)`: little-endian byte-aligned integer
/// - `leb128()`: little-endian base-128 variable-length integer
/// - `su(n)`: fixed-width signed integer
/// - `ns(n)`: non-symmetric range coding helper
///
/// The reader is intentionally simple: it borrows an input slice, maintains a
/// byte cursor plus a bit offset inside the current byte, and exposes methods
/// that match the syntax names from the specification as closely as possible.
///
/// Bit ordering:
///
/// AV1 reads bits MSB-first inside each byte. If the current byte is
/// `0b1011_0010`, the read order is `1, 0, 1, 1, 0, 0, 1, 0`.
///
/// References:
///
/// - AV1 specification, Section 4.10 "Bitstream data syntax"
/// - AV1 specification, Section 5 "Syntax structures"
/// - LEB128 background: DWARF Appendix C and
///   <https://en.wikipedia.org/wiki/LEB128>
pub struct Buffer<'a> {
    buf: &'a [u8],
    /// Current byte index into `buf`.
    index: usize,
    /// Bit offset within the current byte (0 = MSB).
    bit_pos: usize,
}

impl<'a> Buffer<'a> {
    /// Construct a reader over a borrowed byte slice.
    ///
    /// The initial cursor points at the first bit of the first byte:
    /// `index = 0`, `bit_pos = 0`.
    pub fn from_slice(buf: &'a [u8]) -> Self {
        Self {
            buf,
            index: 0,
            bit_pos: 0,
        }
    }

    /// Skip `n` bits without returning a value.
    ///
    /// This is conceptually identical to calling [`get_bit`](Self::get_bit)
    /// `n` times and discarding the result, but it avoids repeated boolean
    /// materialization and keeps the intent explicit when the syntax says to
    /// "ignore" or "skip" reserved bits.
    pub fn seek_bits(&mut self, cut: usize) {
        for _ in 0..cut {
            self.advance();
        }
    }

    /// Read `count` bytes as a slice. Requires byte alignment.
    ///
    /// This method does not copy data. It advances the byte cursor and returns
    /// a borrowed subslice into the original buffer.
    ///
    /// Byte alignment is required because AV1 syntax only permits raw byte
    /// reads at whole-byte boundaries. If `bit_pos != 0`, the caller would be
    /// asking for a slice that starts in the middle of a byte, which cannot be
    /// represented as `&[u8]` without additional packing logic.
    pub fn get_bytes(&mut self, count: usize) -> &[u8] {
        assert_eq!(self.bit_pos, 0, "get_bytes requires byte alignment");
        self.index += count;
        &self.buf[self.index - count..self.index]
    }

    /// Read one bit and return it as a boolean.
    ///
    /// Internally this extracts bit `(7 - bit_pos)` from the current byte, then
    /// advances the cursor by one bit.
    pub fn get_bit(&mut self) -> bool {
        self.next()
    }

    /// f(n): read `count` bits MSB-first as an unsigned integer.
    ///
    /// AV1 spec Section 4.10.2 - f(n).
    ///
    /// Algorithm:
    ///
    /// 1. Read one bit at a time in stream order.
    /// 2. Shift each bit into its numeric position in the result.
    /// 3. The first bit read becomes the highest-order bit of the returned
    ///    value, and the last bit read becomes the lowest-order bit.
    ///
    /// For example, if the next four bits are `1 0 1 1`, the result is:
    ///
    /// `1<<3 | 0<<2 | 1<<1 | 1<<0 = 0b1011 = 11`
    ///
    /// Cross-byte example:
    ///
    /// Suppose the unread stream is:
    ///
    /// - byte 0 = `1010_1011`
    /// - byte 1 = `1100_1101`
    ///
    /// Calling `get_bits(12)` reads:
    ///
    /// - first 8 bits from byte 0: `1010_1011`
    /// - next 4 bits from byte 1:  `1100`
    ///
    /// Concatenating them in read order yields:
    ///
    /// `1010_1011_1100 = 0xABC`
    ///
    /// This is why the implementation ORs each bit into
    /// `(count - i - 1)`: it reconstructs the integer exactly as the bitstring
    /// appears in the specification.
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
    ///
    /// AV1 `uvlc()` uses a prefix code closely related to Exp-Golomb coding:
    ///
    /// - count the number of leading zero bits, `lz`
    /// - consume the terminating `1`
    /// - read `lz` payload bits
    /// - return `payload + 2^lz - 1`
    ///
    /// Example:
    ///
    /// - Bit pattern `1`      -> `lz=0`, payload bits=`""`,   value=`0`
    /// - Bit pattern `010`    -> `lz=1`, payload bits=`0`,    value=`1`
    /// - Bit pattern `011`    -> `lz=1`, payload bits=`1`,    value=`2`
    /// - Bit pattern `00110`  -> `lz=2`, payload bits=`10`,   value=`5`
    ///
    /// Worked example for `00110`:
    ///
    /// - leading zeros: `00` -> `lz = 2`
    /// - stop bit: `1`
    /// - payload: `10` -> decimal `2`
    /// - value: `2 + 2^2 - 1 = 5`
    ///
    /// The `2^lz - 1` offset makes codes of different prefix lengths map to
    /// contiguous integer ranges.
    ///
    /// Per the spec, if `lz >= 32`, the decoder returns `0xFFFF_FFFF`.
    ///
    /// Related background: this is closely related to unsigned Exp-Golomb
    /// coding, but AV1 defines the exact mapping normatively in spec
    /// Section 4.10.3.
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

    /// le(n): unsigned little-endian `count`-byte integer.
    ///
    /// AV1 spec Section 4.10.4 - le(n).
    ///
    /// Requires byte alignment because the syntax is defined over complete
    /// bytes, not arbitrary bit positions.
    ///
    /// The implementation reads bytes in stream order and places byte `i` into
    /// bit range `[8*i, 8*i+7]` of the result:
    ///
    /// `value = b0 + (b1 << 8) + (b2 << 16) + ...`
    ///
    /// So bytes `[0x34, 0x12]` decode to `0x1234`.
    ///
    /// Worked example:
    ///
    /// - first byte  read: `0x78`
    /// - second byte read: `0x56`
    /// - third byte  read: `0x34`
    /// - fourth byte read: `0x12`
    ///
    /// Then:
    ///
    /// `0x78 + (0x56 << 8) + (0x34 << 16) + (0x12 << 24) = 0x12345678`
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
    ///
    /// LEB128 stores an integer in 7-bit groups:
    ///
    /// - bit 7 of each byte is the continuation flag
    /// - bits 0..6 carry payload
    /// - the first byte contains the least-significant 7 payload bits
    ///
    /// Numerically this means:
    ///
    /// `value = group0 << 0 | group1 << 7 | group2 << 14 | ...`
    ///
    /// Example:
    ///
    /// - `[0x05]` -> `5`
    /// - `[0x80, 0x01]` -> `128`
    /// - `[0xAC, 0x02]` -> `300`
    ///
    /// Worked example for `[0xAC, 0x02]`:
    ///
    /// - `0xAC = 1010_1100`
    ///   - continuation = `1`
    ///   - payload       = `0x2C = 44`
    /// - `0x02 = 0000_0010`
    ///   - continuation = `0`
    ///   - payload       = `0x02 = 2`
    ///
    /// Reassemble in little-endian 7-bit groups:
    ///
    /// `44 << 0 | 2 << 7 = 44 + 256 = 300`
    ///
    /// The implementation stops when it encounters a byte whose continuation
    /// flag is `0`, or after 8 bytes, matching the AV1 spec limit.
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
    ///
    /// AV1 defines `su(n)` as a fixed-width signed integer encoded in two's
    /// complement over exactly `n` bits.
    ///
    /// Decoding strategy:
    ///
    /// 1. Read the `n` bits as an unsigned integer.
    /// 2. Inspect the top bit (`1 << (n - 1)`), which is the sign bit.
    /// 3. If the sign bit is clear, the value is already non-negative.
    /// 4. If the sign bit is set, subtract `2^n` to sign-extend into `i32`.
    ///
    /// Example for `n = 4`:
    ///
    /// - `0011` -> `3`
    /// - `1100` -> `12 - 16 = -4`
    ///
    /// Another way to see the negative case:
    ///
    /// - `n = 4` means the representable range is `[-8, 7]`
    /// - raw unsigned `1100` is `12`
    /// - because the sign bit is set, interpret it modulo `2^4 = 16`
    /// - `12 - 16 = -4`
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
    ///
    /// Motivation:
    ///
    /// When `n` is not a power of two, a fixed-width code wastes states.
    /// For example, values in `[0, 4]` need 5 states, but 3 bits represent
    /// 8 states. AV1's `ns(n)` removes that waste by using:
    ///
    /// - a short code for the first `m` values
    /// - a long code for the remaining `n - m` values
    ///
    /// where:
    ///
    /// - `w = ceil(log2(n))`
    /// - `m = 2^w - n`
    ///
    /// Decoding algorithm:
    ///
    /// 1. Read `w - 1` bits to get `v`.
    /// 2. If `v < m`, return `v`.
    /// 3. Otherwise read one extra bit `b` and return `(v << 1) - m + b`.
    ///
    /// This partitions the code space so exactly `n` output values are
    /// generated, while keeping the code as close as possible to fixed-width.
    ///
    /// Example for `n = 5`:
    ///
    /// - `w = 3`, `m = 8 - 5 = 3`
    /// - values `0,1,2` use 2 bits: `00, 01, 10`
    /// - values `3,4` use 3 bits: `110, 111`
    ///
    /// Worked decode examples for `n = 5`:
    ///
    /// - input `01`
    ///   - read `w - 1 = 2` bits -> `v = 1`
    ///   - `v < m` (`1 < 3`) -> return `1`
    ///
    /// - input `110`
    ///   - read first 2 bits -> `v = 3`
    ///   - `v >= m` (`3 >= 3`) -> read one extra bit `0`
    ///   - return `(3 << 1) - 3 + 0 = 3`
    ///
    /// - input `111`
    ///   - read first 2 bits -> `v = 3`
    ///   - extra bit = `1`
    ///   - return `(3 << 1) - 3 + 1 = 4`
    ///
    /// Reference: the AV1 spec defines this directly in Section 4.10.7; the
    /// same idea is also known as truncated binary coding in information
    /// theory; see also <https://en.wikipedia.org/wiki/Truncated_binary_encoding>.
    pub fn get_ns(&mut self, n: u32) -> u32 {
        if n <= 1 {
            return 0;
        }
        // `leading_zeros` gives us ceil(log2(n)) in integer form.
        let w = (32 - n.leading_zeros()) as usize;
        // `m` is the number of values that can use the short `(w - 1)`-bit form.
        let m = (1u32 << w) - n;
        let v = self.get_bits(w - 1);
        if v < m {
            v
        } else {
            let extra_bit = self.get_bit() as u32;
            (v << 1) - m + extra_bit
        }
    }

    /// Returns `true` if the cursor is at a byte boundary.
    ///
    /// This simply means no partial bits of the current byte have been
    /// consumed, i.e. `bit_pos == 0`.
    pub fn is_byte_aligned(&self) -> bool {
        self.bit_pos == 0
    }

    /// Advance to the next byte boundary, discarding any remaining bits in the
    /// current byte (trailing_bits padding).
    ///
    /// This is commonly used after parsing AV1 payloads that end in
    /// `trailing_bits()`: a single `1` bit followed by enough `0` bits to
    /// complete the byte.
    ///
    /// Example:
    ///
    /// If 3 bits of the current byte have already been consumed, then
    /// `bit_pos = 3` and `byte_align()` skips `8 - 3 = 5` bits so that the next
    /// read starts at the next byte.
    pub fn byte_align(&mut self) {
        if self.bit_pos != 0 {
            self.seek_bits(8 - self.bit_pos);
        }
    }

    /// Returns the number of bytes remaining from the current byte index.
    ///
    /// This is intentionally byte-granular. If the cursor is mid-byte, the
    /// partially consumed current byte still counts as remaining because future
    /// bit reads can continue from it.
    pub fn bytes_remaining(&self) -> usize {
        if self.index >= self.buf.len() {
            return 0;
        }
        self.buf.len() - self.index
    }

    /// Returns the number of bytes consumed so far, rounded up.
    ///
    /// Rounding up is useful when enforcing AV1 OBU boundaries, because having
    /// consumed even one bit from a byte means that byte is no longer available
    /// to subsequent syntax elements.
    pub fn bytes_consumed(&self) -> usize {
        self.index + if self.bit_pos > 0 { 1 } else { 0 }
    }
}

impl<'a> Buffer<'a> {
    /// Advance the internal cursor by one bit.
    ///
    /// The cursor is stored as `(index, bit_pos)` where `bit_pos` is in
    /// `[0, 7]`. Advancing increments `bit_pos`; when it reaches `8`, we wrap
    /// to the next byte and reset `bit_pos` back to `0`.
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
    ///
    /// Because AV1 is MSB-first, the next unread bit in the current byte is
    /// located at position `7 - bit_pos`.
    ///
    /// Example with current byte `0b1011_0010`:
    ///
    /// - `bit_pos = 0` -> shift `7` -> read `1`
    /// - `bit_pos = 1` -> shift `6` -> read `0`
    /// - `bit_pos = 2` -> shift `5` -> read `1`
    ///
    /// The expression `curr_byte & (1 << shift)` isolates that bit, and the
    /// final right-shift normalizes it to `0` or `1`.
    ///
    /// Bit diagram for `curr_byte = 1011_0010`:
    ///
    /// ```text
    /// bit index:  7 6 5 4 3 2 1 0
    /// value:      1 0 1 1 0 0 1 0
    ///               ^ current bit when bit_pos = 0
    ///                 ^ current bit when bit_pos = 1
    ///                   ^ current bit when bit_pos = 2
    /// ```
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
        let mut buf = Buffer::from_slice(&data);
        assert_eq!(buf.get_bit(), true); // bit 7 = 1
        assert_eq!(buf.get_bit(), false); // bit 6 = 0
        assert_eq!(buf.get_bit(), true); // bit 5 = 1
        assert_eq!(buf.get_bit(), true); // bit 4 = 1
        assert_eq!(buf.get_bit(), false); // bit 3 = 0
        assert_eq!(buf.get_bit(), false); // bit 2 = 0
        assert_eq!(buf.get_bit(), true); // bit 1 = 1
        assert_eq!(buf.get_bit(), false); // bit 0 = 0
    }

    #[test]
    fn test_get_bits() {
        let data = [0xABu8, 0xCDu8]; // 10101011 11001101
        let mut buf = Buffer::from_slice(&data);
        assert_eq!(buf.get_bits(4), 0xA); // 1010
        assert_eq!(buf.get_bits(4), 0xB); // 1011
        assert_eq!(buf.get_bits(8), 0xCD); // 11001101
    }

    #[test]
    fn test_get_leb128() {
        // Single-byte LEB128: 5
        let data = [0x05u8];
        let mut buf = Buffer::from_slice(&data);
        assert_eq!(buf.get_leb128(), 5);

        // Two-byte LEB128: 128 encoded as [0x80, 0x01]
        let data2 = [0x80u8, 0x01u8];
        let mut buf2 = Buffer::from_slice(&data2);
        assert_eq!(buf2.get_leb128(), 128);
    }

    #[test]
    fn test_get_su() {
        // su(4): read 1100 = 12; sign bit set, so result = 12 - 16 = -4
        let data = [0b1100_0000u8];
        let mut buf = Buffer::from_slice(&data);
        assert_eq!(buf.get_su(4), -4);
    }

    #[test]
    fn test_get_ns() {
        // ns(4): n=4, w=3, m=(1<<3)-4=4.
        // m=4 means all 2-bit values (0–3) are smaller than m and are returned
        // directly without reading an extra bit.
        let data = [0b00_01_10_11u8];
        let mut buf = Buffer::from_slice(&data);
        assert_eq!(buf.get_ns(4), 0); // 00 → 0
        assert_eq!(buf.get_ns(4), 1); // 01 → 1
        assert_eq!(buf.get_ns(4), 2); // 10 → 2
        assert_eq!(buf.get_ns(4), 3); // 11 → 3 (still < m=4, no extra bit)
    }

    #[test]
    fn test_byte_align() {
        let data = [0xFFu8, 0xAAu8];
        let mut buf = Buffer::from_slice(&data);
        buf.get_bits(3);
        assert!(!buf.is_byte_aligned());
        buf.byte_align();
        assert!(buf.is_byte_aligned());
        assert_eq!(buf.get_bits(8), 0xAA);
    }
}
