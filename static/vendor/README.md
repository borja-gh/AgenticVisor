# Bundled libraries

`cm-json.js` is a local bundle of CodeMirror 6, used by the configuration and
skill editors. It is embedded in the binary, so the program makes no network
requests to load it.

- codemirror 6.0.2
- @codemirror/lint 6.9.7
- @codemirror/lang-json 6.0.2
- @codemirror/lang-markdown 6.5.2
- @codemirror/theme-one-dark (dark theme)

License: MIT (CodeMirror and its packages).

To rebuild it, `cm-json.src.js` is the entry point. Bundle it with esbuild:
`esbuild cm-json.src.js --bundle --format=iife --minify --outfile=cm-json.js`.
