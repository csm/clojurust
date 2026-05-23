/* clj.rs interactive REPL — loaded by mdBook as additional-js.
 *
 * This file runs as a regular (non-module) script. The WASM module is loaded
 * lazily via dynamic import() once the page is idle, so page content renders
 * without waiting for the binary.
 *
 * Keyboard contract:
 *   Enter        — insert newline
 *   Tab          — insert two spaces (indent)
 *   Shift+Enter  — evaluate
 *   ↑ / ↓        — history navigation (when cursor is on first/last line)
 */

(function () {
  'use strict';

  // Absolute path — works from any page depth once deployed to clj.rs root.
  var WASM_JS = '/wasm/cljrs_wasm.js';

  var repl = null;
  var history = [];
  var historyIdx = -1;
  var liveDraft = '';
  var dividerInserted = false;

  // ── DOM helpers ──────────────────────────────────────────────────────────

  function el(tag, attrs, children) {
    var node = document.createElement(tag);
    if (attrs) {
      Object.keys(attrs).forEach(function (k) {
        if (k === 'className') node.className = attrs[k];
        else if (k === 'textContent') node.textContent = attrs[k];
        else node.setAttribute(k, attrs[k]);
      });
    }
    if (children) {
      children.forEach(function (c) { node.appendChild(c); });
    }
    return node;
  }

  // ── Build the fixed bottom bar ───────────────────────────────────────────

  function buildBar() {
    var prompt = el('span', { id: 'cljrs-repl-prompt', textContent: 'user=> ' });

    var input = el('textarea', {
      id: 'cljrs-repl-input',
      rows: '1',
      disabled: 'true',
      autocomplete: 'off',
      spellcheck: 'false',
      placeholder: 'Loading REPL…',
      'aria-label': 'Clojure REPL input',
    });

    var status = el('span', { id: 'cljrs-repl-status', textContent: 'loading…' });

    var bar = el('div', { id: 'cljrs-repl-bar' }, [prompt, input, status]);
    document.body.appendChild(bar);

    // Reserve space so the bar does not overlap content.
    document.body.style.paddingBottom = '52px';

    return { input: input, status: status };
  }

  // ── Content area helpers ─────────────────────────────────────────────────

  function contentEl() {
    // mdBook wraps the rendered markdown in .content; fall back to <main>.
    return document.querySelector('.content') || document.querySelector('main') || document.body;
  }

  function ensureDivider() {
    if (dividerInserted) return;
    dividerInserted = true;
    contentEl().appendChild(el('div', { className: 'cljrs-repl-divider' }));
  }

  function appendEntry(code, printOut, result, isError) {
    ensureDivider();

    var inDiv = el('div', { className: 'cljrs-repl-in', textContent: code });
    var children = [inDiv];

    if (printOut) {
      children.push(el('div', { className: 'cljrs-repl-print', textContent: printOut }));
    }
    if (result !== '') {
      children.push(el('div', {
        className: isError ? 'cljrs-repl-err' : 'cljrs-repl-out',
        textContent: result,
      }));
    }

    var entry = el('div', { className: 'cljrs-repl-entry' }, children);
    contentEl().appendChild(entry);
    entry.scrollIntoView({ behavior: 'smooth', block: 'end' });
  }

  // ── Auto-resize textarea ─────────────────────────────────────────────────

  function resize(textarea) {
    textarea.style.height = 'auto';
    textarea.style.height = Math.min(textarea.scrollHeight, 200) + 'px';
    // Adjust body padding so bar height changes don't obscure content.
    var bar = document.getElementById('cljrs-repl-bar');
    if (bar) document.body.style.paddingBottom = (bar.offsetHeight + 4) + 'px';
  }

  // ── Cursor position helpers (for history nav) ────────────────────────────

  function isOnFirstLine(textarea) {
    return textarea.value.lastIndexOf('\n', textarea.selectionStart - 1) === -1;
  }

  function isOnLastLine(textarea) {
    return textarea.value.indexOf('\n', textarea.selectionStart) === -1;
  }

  // ── Keyboard handling ────────────────────────────────────────────────────

  function onKeyDown(e, textarea) {
    // Tab → two-space indent.
    if (e.key === 'Tab') {
      e.preventDefault();
      var s = textarea.selectionStart;
      textarea.value = textarea.value.slice(0, s) + '  ' + textarea.value.slice(s);
      textarea.selectionStart = textarea.selectionEnd = s + 2;
      resize(textarea);
      return;
    }

    // Shift+Enter → evaluate.
    if (e.key === 'Enter' && e.shiftKey) {
      e.preventDefault();
      var code = textarea.value;
      if (!code.trim() || !repl) return;

      if (history[history.length - 1] !== code) history.push(code);
      historyIdx = -1;
      liveDraft = '';

      var out = repl.eval(code);
      appendEntry(code, out.output, out.result, out.is_error);

      textarea.value = '';
      resize(textarea);
      return;
    }

    // ↑ — history back (only when cursor is on the first line).
    if (e.key === 'ArrowUp' && isOnFirstLine(textarea)) {
      if (history.length === 0) return;
      e.preventDefault();
      if (historyIdx === -1) {
        liveDraft = textarea.value;
        historyIdx = history.length - 1;
      } else if (historyIdx > 0) {
        historyIdx--;
      }
      textarea.value = history[historyIdx];
      resize(textarea);
      textarea.selectionStart = textarea.selectionEnd = textarea.value.length;
      return;
    }

    // ↓ — history forward (only when cursor is on the last line).
    if (e.key === 'ArrowDown' && isOnLastLine(textarea)) {
      if (historyIdx === -1) return;
      e.preventDefault();
      if (historyIdx < history.length - 1) {
        historyIdx++;
        textarea.value = history[historyIdx];
      } else {
        historyIdx = -1;
        textarea.value = liveDraft;
      }
      resize(textarea);
      textarea.selectionStart = textarea.selectionEnd = textarea.value.length;
      return;
    }
  }

  // ── Lazy WASM load ───────────────────────────────────────────────────────

  function loadWasm(input, status) {
    // Dynamic import() works in non-module scripts in all modern browsers.
    import(WASM_JS).then(function (mod) {
      return mod.default().then(function () { return mod; });
    }).then(function (mod) {
      repl = new mod.Repl();
      input.disabled = false;
      input.placeholder = '(+ 1 2) — Shift+Enter to eval, Tab to indent, ↑↓ history';
      status.textContent = '● ready';
      status.className = 'ready';
      input.focus();
    }).catch(function (err) {
      status.textContent = '✕ failed';
      status.className = 'error';
      status.title = String(err);
      console.error('[cljrs-wasm]', err);
    });
  }

  // ── Init ─────────────────────────────────────────────────────────────────

  function init() {
    var els = buildBar();
    var input = els.input;
    var status = els.status;

    input.addEventListener('input', function () { resize(input); });
    input.addEventListener('keydown', function (e) { onKeyDown(e, input); });

    // Kick off WASM load when the browser is idle so the page paints first.
    if (typeof requestIdleCallback !== 'undefined') {
      requestIdleCallback(function () { loadWasm(input, status); }, { timeout: 4000 });
    } else {
      setTimeout(function () { loadWasm(input, status); }, 250);
    }
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
}());
