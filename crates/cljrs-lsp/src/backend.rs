//! The tower-lsp [`LanguageServer`] implementation and stdio entry points.

use std::sync::RwLock;

use dashmap::DashMap;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server, jsonrpc::Result};

use crate::analysis;
use crate::document::Document;
use crate::line_index::OffsetEncoding;

/// The clojurust language server backend.
pub struct Backend {
    client: Client,
    docs: DashMap<Url, Document>,
    /// Position encoding negotiated in `initialize` (UTF-16 until then).
    encoding: RwLock<OffsetEncoding>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            docs: DashMap::new(),
            encoding: RwLock::new(OffsetEncoding::Utf16),
        }
    }

    fn encoding(&self) -> OffsetEncoding {
        *self.encoding.read().unwrap()
    }

    /// Re-analyze a document and publish its diagnostics.
    async fn refresh(&self, uri: Url) {
        // Snapshot the text and drop the store guard before any `.await`.
        let Some((text, version)) = self.docs.get(&uri).map(|d| (d.text.clone(), d.version)) else {
            return;
        };

        let analysis = analysis::run(&text, uri.as_str(), self.encoding());

        if let Some(mut doc) = self.docs.get_mut(&uri) {
            doc.symbols = analysis.symbols;
        }

        self.client
            .publish_diagnostics(uri, analysis.diagnostics, Some(version))
            .await;
    }
}

/// Choose a position encoding: prefer UTF-8 when the client advertises it.
fn negotiate_encoding(params: &InitializeParams) -> OffsetEncoding {
    let supports_utf8 = params
        .capabilities
        .general
        .as_ref()
        .and_then(|g| g.position_encodings.as_ref())
        .map(|encs| encs.contains(&PositionEncodingKind::UTF8))
        .unwrap_or(false);
    if supports_utf8 {
        OffsetEncoding::Utf8
    } else {
        OffsetEncoding::Utf16
    }
}

fn to_lsp_encoding(enc: OffsetEncoding) -> PositionEncodingKind {
    match enc {
        OffsetEncoding::Utf8 => PositionEncodingKind::UTF8,
        OffsetEncoding::Utf16 => PositionEncodingKind::UTF16,
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let enc = negotiate_encoding(&params);
        *self.encoding.write().unwrap() = enc;

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                position_encoding: Some(to_lsp_encoding(enc)),
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                document_symbol_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "cljrs-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "cljrs-lsp ready")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let td = params.text_document;
        let uri = td.uri.clone();
        self.docs
            .insert(uri.clone(), Document::new(td.text, td.version));
        self.refresh(uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let VersionedTextDocumentIdentifier { uri, version } = params.text_document;
        // FULL sync: the last change carries the entire new document text.
        if let Some(change) = params.content_changes.into_iter().last() {
            match self.docs.get_mut(&uri) {
                Some(mut doc) => {
                    doc.text = change.text;
                    doc.version = version;
                }
                None => {
                    self.docs
                        .insert(uri.clone(), Document::new(change.text, version));
                }
            }
        }
        self.refresh(uri).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.docs.remove(&uri);
        // Clear diagnostics for the closed document.
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let symbols = self
            .docs
            .get(&params.text_document.uri)
            .map(|d| d.symbols.clone())
            .unwrap_or_default();
        Ok(Some(DocumentSymbolResponse::Nested(symbols)))
    }
}

/// Run the language server over stdio until the client disconnects.
pub async fn run_stdio() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}

/// Build a runtime and run [`run_stdio`] to completion. Convenience for callers
/// (e.g. the `cljrs lsp` subcommand) that are not already async.
pub fn run_stdio_blocking() -> std::io::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(run_stdio());
    Ok(())
}
