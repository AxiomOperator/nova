//! Nova surface syntax: lexer, AST, and parser.

pub mod ast;
pub mod lexer;
pub mod parser;

pub use parser::parse_file;
