use crate::codegen::Codegen;
use crate::Error;

#[derive(Default)]
pub struct Es6 {}

impl Codegen for Es6 {
    fn file(&self, _: &mut String) -> Result<(), Error> {
        todo!()
    }
}
