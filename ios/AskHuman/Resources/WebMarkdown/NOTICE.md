# Third-party code in this folder

`markdown-it.min.js` and `mermaid.min.js` are the unmodified distribution files of the npm packages
named in `VERSIONS.txt`, the same ones the Mac app uses. They are copied here by
`ios/Tools/web-markdown/sync.sh`.

- **markdown-it** (MIT): see `LICENSE-markdown-it.txt`. https://github.com/markdown-it/markdown-it
- **Mermaid** (MIT): see `LICENSE-mermaid.txt`. https://github.com/mermaid-js/mermaid
  The minified file also contains the code of Mermaid's own dependencies (the `dependencies` of the
  package at the version in `VERSIONS.txt`); each of them keeps its own license.

`index.html`, `style.css` and `render.js` are part of AskHuman.
