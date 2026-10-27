import { EditorView, basicSetup } from "codemirror";
import { EditorState } from "@codemirror/state";
import { json, jsonParseLinter } from "@codemirror/lang-json";
import { markdown } from "@codemirror/lang-markdown";
import { linter, lintGutter } from "@codemirror/lint";
import { oneDark } from "@codemirror/theme-one-dark";

// Creates a JSON or Markdown editor. Validates JSON live (markers in the gutter)
// and reports each change so the interface can enable the save button.
function createEditor(parent, { doc = "", mode = "json", dark = false, onChange = () => {} }) {
  const language = mode === "json" ? json() : markdown();
  const extensions = [
    basicSetup,
    language,
    EditorView.lineWrapping,
    EditorView.updateListener.of(update => { if (update.docChanged) onChange(); }),
    EditorView.theme({
      "&": { height: "100%", fontSize: "13px" },
      ".cm-scroller": { fontFamily: "ui-monospace, 'SF Mono', Menlo, monospace" },
    }),
  ];
  if (mode === "json") extensions.push(linter(jsonParseLinter()), lintGutter());
  if (dark) extensions.push(oneDark);
  const view = new EditorView({ parent, state: EditorState.create({ doc, extensions }) });
  return {
    getValue: () => view.state.doc.toString(),
    setValue: (text) => view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } }),
    focus: () => view.focus(),
    destroy: () => view.destroy(),
  };
}

window.CodeEditor = { createEditor };
