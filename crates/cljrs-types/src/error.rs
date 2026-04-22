// Fields are read by the thiserror/miette derive macros; suppress false-positive
// unused_assignments warnings until callers land in later phases.
#![allow(unused)]

/// The unified error/diagnostic type for all clojurust subsystems.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum CljxError {
    #[error("read error: {message}")]
    #[diagnostic(code(cljrs::read))]
    ReadError {
        message: String,
        #[label("here")]
        span: Option<miette::SourceSpan>,
        #[source_code]
        src: miette::NamedSource<String>,
    },

    #[error("eval error: {message}")]
    #[diagnostic(code(cljrs::eval))]
    EvalError {
        message: String,
        #[label("here")]
        span: Option<miette::SourceSpan>,
        #[source_code]
        src: miette::NamedSource<String>,
    },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {message}")]
    SerializationError { message: String },
}

pub type CljxResult<T> = Result<T, CljxError>;
