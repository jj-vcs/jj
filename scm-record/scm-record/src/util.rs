pub trait UsizeExt {
    fn unwrap_isize(self) -> isize;
    fn clamp_into_u16(self) -> u16;
}

impl UsizeExt for usize {
    fn unwrap_isize(self) -> isize {
        isize::try_from(self).unwrap()
    }

    fn clamp_into_u16(self) -> u16 {
        if self > u16::MAX.into() {
            u16::MAX
        } else {
            self.try_into().unwrap()
        }
    }
}

pub trait IsizeExt {
    fn unwrap_usize(self) -> usize;
    #[allow(dead_code)]
    fn clamp_into_u16(self) -> u16;
    fn clamp_into_usize(self) -> usize;
}

impl IsizeExt for isize {
    fn unwrap_usize(self) -> usize {
        usize::try_from(self).unwrap()
    }

    fn clamp_into_u16(self) -> u16 {
        if self < 0 {
            0
        } else {
            self.try_into().unwrap_or(u16::MAX)
        }
    }

    fn clamp_into_usize(self) -> usize {
        if self < 0 {
            0
        } else {
            self.try_into().unwrap_or(usize::MAX)
        }
    }
}
