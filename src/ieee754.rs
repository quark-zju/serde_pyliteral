pub(crate) trait IeeeFloat<const E: u16, const F: u16> {
    /// 2-based exponent. If the number can be written as `1.__ * (2 ** x)`,
    /// then this function returns the `x`.
    fn exponent(&self) -> i16 {
        let bits = self.to_u64_bits();
        (((bits >> F) & ((1 << E) - 1)) as i16) + 1 - (1 << (E - 1))
    }

    /// Whether scientific notation is more proper to display the number.
    fn should_use_scientific_notation(&self) -> bool {
        self.exponent().abs() > (F as i16)
    }

    /// Format the value to string suitable for human to read.
    fn to_human_string(&self) -> String
    where
        Self: std::fmt::LowerExp + std::fmt::Display,
    {
        let mut s = if self.should_use_scientific_notation() {
            format!("{:e}", self)
        } else {
            format!("{}", self)
        };
        // If it looks like an integer, append '.' to make it an explicit float.
        if s.as_bytes().iter().all(|&b| b >= b'0' && b <= b'9') {
            s.push('.');
        }
        s
    }

    fn to_u64_bits(&self) -> u64;
}

impl IeeeFloat<8, 23> for f32 {
    fn to_u64_bits(&self) -> u64 {
        self.to_bits() as u64
    }
}

impl IeeeFloat<11, 52> for f64 {
    fn to_u64_bits(&self) -> u64 {
        self.to_bits()
    }
}
