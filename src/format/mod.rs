use crate::debian::Pkg;
use crate::errors::*;
use crate::format::parse::{Parser, RawChunk};

pub mod human;
pub mod json;
mod parse;

enum Chunk {
    Raw(String),
    Package,
    License,
    Repository,
}

pub struct Pattern(Vec<Chunk>);

impl Pattern {
    pub fn new(format: &str) -> Result<Pattern, Error> {
        let mut chunks = vec![];

        for raw in Parser::new(format) {
            let chunk = match raw {
                RawChunk::Text(text) => Chunk::Raw(text.to_owned()),
                RawChunk::Argument("p") => Chunk::Package,
                RawChunk::Argument("l") => Chunk::License,
                RawChunk::Argument("r") => Chunk::Repository,
                RawChunk::Argument(ref a) => {
                    return Err(anyhow!("unsupported pattern `{}`", a));
                }
                RawChunk::Error(err) => return Err(anyhow!("{}", err)),
            };
            chunks.push(chunk);
        }

        Ok(Pattern(chunks))
    }
}
