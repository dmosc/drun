#[derive(Clone, Copy)]
pub enum NetworkPolicy {
    Packages,
    Full,
    None,
}

impl NetworkPolicy {
    pub fn from_opt_str(s: Option<&str>) -> Self {
        match s {
            Some("full") => Self::Full,
            Some("none") => Self::None,
            _ => Self::Packages,
        }
    }
}
