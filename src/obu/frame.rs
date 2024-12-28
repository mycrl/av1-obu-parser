use crate::buffer::Buffer;

use super::{ObuContext, ObuError};

#[derive(Debug, Clone)]
pub struct Frame {}

impl Frame {
    pub fn decode(ctx: &mut ObuContext, buf: &mut Buffer) -> Result<Self, ObuError> {
        todo!()
    }
}
