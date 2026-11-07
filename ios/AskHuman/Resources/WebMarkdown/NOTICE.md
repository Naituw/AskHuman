# Third-party code in this folder

`markdown-it.min.js` and `mermaid.min.js` are the unmodified distribution files of the npm packages
named in `VERSIONS.txt`, the same ones the Mac app uses. They are copied here by
`ios/Tools/web-markdown/sync.sh`.

- **markdown-it** (MIT): see `LICENSE-markdown-it.txt`. https://github.com/markdown-it/markdown-it
- **Mermaid** (MIT): see `LICENSE-mermaid.txt`. https://github.com/mermaid-js/mermaid
  The minified file also contains the code of Mermaid's own dependencies (the `dependencies` of the
  package at the version in `VERSIONS.txt`); each of them keeps its own license.

`annotations.js` is AskHuman's own annotation engine, built from `src/lib/annotations.ts` (the file the Mac popup
imports) by `ios/Tools/web-markdown/build-annotations.sh`; its first line names the source it was built from.

`index.html`, `style.css`, `render.js` and `notes.js` are part of AskHuman.
