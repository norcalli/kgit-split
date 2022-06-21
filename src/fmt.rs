use std::fmt::{self, Debug, Display};

pub struct OrDisplay<T: Display, U>(Option<T>, U);

impl<T: Display, U: Display> Display for OrDisplay<T, U> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match &self.0 {
            Some(t) => t.fmt(fmt),
            None => self.1.fmt(fmt),
        }
    }
}

pub trait OrDisplayExt<T: Display> {
    fn or_display<U: Display>(&self, u: U) -> OrDisplay<&T, U>;

    fn into_or_display<U: Display>(self, u: U) -> OrDisplay<T, U>;
}

impl<T: Display> OrDisplayExt<T> for Option<T> {
    fn or_display<U: Display>(&self, u: U) -> OrDisplay<&T, U> {
        OrDisplay(self.as_ref(), u)
    }

    fn into_or_display<U: Display>(self, u: U) -> OrDisplay<T, U> {
        OrDisplay(self, u)
    }
}

pub struct DisplayViaDebug<T: Debug>(pub T);

impl<T: Debug> std::fmt::Display for DisplayViaDebug<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)?;
        Ok(())
    }
}

pub struct ThousandsUnsigned(pub u64);

impl std::fmt::Display for ThousandsUnsigned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0 < 1000 {
            return write!(f, "{}", self.0);
        }
        let mut v = self.0;
        let mut buckets = [0u16; 7];
        let mut i = 0;
        while v > 0 {
            buckets[i] = (v % 1000) as u16;
            v /= 1000;
            i += 1;
        }
        write!(f, "{}", buckets[i - 1])?;
        for i in (0..i - 1).rev() {
            write!(f, " {:03}", buckets[i])?;
        }
        Ok(())
    }
}

pub struct ThousandsSigned(pub i64);

impl std::fmt::Display for ThousandsSigned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0 < 0 {
            write!(f, "-")?;
        }
        write!(f, "{}", ThousandsUnsigned(self.0.abs() as u64))
    }
}

pub struct ThousandsFloat(pub f64);

impl std::fmt::Display for ThousandsFloat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use std::io::Write; // For printing to &[u8]
        if self.0 < 0.0 {
            write!(f, "-")?;
        } else if f.sign_plus() {
            write!(f, "+")?;
        }
        let absolute = self.0.abs();
        write!(f, "{}", ThousandsUnsigned(absolute as u64))?;
        // This might be a silly optimization, but why not, I guess.
        // f64 only has 16 decimals of precision but overshoot for safety, I guess.
        let mut decimal_buffer = [0u8; 32];
        // NOTE fract() *might* slightly change the value.
        if let Some(prec) = f.precision() {
            assert!(prec <= 30);
            write!(&mut decimal_buffer[..], "{:.*}", prec, absolute.fract())
        } else {
            write!(&mut decimal_buffer[..], "{}", absolute.fract())
        }
        .map_err(|_| std::fmt::Error)?;
        let mut it = decimal_buffer.iter();
        if let Some(start) = it.position(|&c| c == b'.') {
            let end = start + it.position(|&c| c == 0).unwrap();
            let decimals = std::str::from_utf8(&decimal_buffer[start..=end]).unwrap();
            // Includes the period
            write!(f, "{}", decimals)?;
        }
        Ok(())
    }
}
