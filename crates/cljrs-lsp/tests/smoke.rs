//! In-process smoke test: drive the language server through its JSON-RPC service
//! and check that `initialize` + `didOpen` + `documentSymbol` work end to end.

use cljrs_lsp::Backend;
use futures::StreamExt;
use serde_json::json;
use tower::Service;
use tower::ServiceExt; // for `.ready()`
use tower_lsp::LspService;
use tower_lsp::jsonrpc::Request;

#[tokio::test]
async fn initialize_open_and_document_symbol() {
    let (mut service, socket) = LspService::new(Backend::new);

    // Drain server→client messages (e.g. publishDiagnostics, logMessage) so the
    // server's sends never block on a full socket channel.
    let _drain = tokio::spawn(async move {
        let mut socket = socket;
        while socket.next().await.is_some() {}
    });

    // initialize
    let init = Request::build("initialize")
        .params(json!({ "capabilities": {} }))
        .id(1)
        .finish();
    let resp = service
        .ready()
        .await
        .unwrap()
        .call(init)
        .await
        .unwrap()
        .expect("initialize response");
    let (_, result) = resp.into_parts();
    let value = result.expect("initialize result");
    // The server must advertise document-symbol support.
    assert_eq!(value["capabilities"]["documentSymbolProvider"], json!(true));

    // initialized (notification — no response)
    let initialized = Request::build("initialized").params(json!({})).finish();
    let _ = service
        .ready()
        .await
        .unwrap()
        .call(initialized)
        .await
        .unwrap();

    // didOpen a document with a good def and a stray closing paren.
    let did_open = Request::build("textDocument/didOpen")
        .params(json!({
            "textDocument": {
                "uri": "file:///t.cljrs",
                "languageId": "clojure",
                "version": 1,
                "text": "(defn f [x] x)\n)\n"
            }
        }))
        .finish();
    let _ = service.ready().await.unwrap().call(did_open).await.unwrap();

    // documentSymbol should return the `f` definition.
    let doc_sym = Request::build("textDocument/documentSymbol")
        .params(json!({ "textDocument": { "uri": "file:///t.cljrs" } }))
        .id(2)
        .finish();
    let resp = service
        .ready()
        .await
        .unwrap()
        .call(doc_sym)
        .await
        .unwrap()
        .expect("documentSymbol response");
    let (_, result) = resp.into_parts();
    let value = result.expect("documentSymbol result");
    let symbols = value.as_array().expect("array of symbols");
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0]["name"], json!("f"));
}
