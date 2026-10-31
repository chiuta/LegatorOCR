/// One recognised word, in pixel coordinates of the processed page
/// (origin top-left, y down).
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Engine confidence 0..100 (negative = unknown).
    pub conf: f32,
    /// Index of the text line the word belongs to (for reading order).
    pub line: u32,
}
