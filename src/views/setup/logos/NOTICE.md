# Marks of the badge tiles

The setup wizard (`src/views/setup`) and the Agents and Channels tabs of the settings window show
every Agent and every remote-answering channel on a small tile with the brand's mark on it, drawn
the way the iOS app draws an Agent
(`ios/AskHuman/Design/AgentBadge.swift`) and the way the README shows the products. The marks only
identify the product a tile stands for; the names and marks belong to their owners and AskHuman is
not affiliated with any of them.

## Agents

Cursor, Claude, Codex, Grok, Pi and Antigravity use the SVG sources of the iOS app unchanged:
`ios/Tools/agent-logos/*.svg` (sources and licenses in that folder's `NOTICE.md`). `brandBadges.ts`
imports them from there, so a logo exists once and both apps change together.

## This folder

| File | Mark | Source | License |
|---|---|---|---|
| `telegram.svg` | Telegram | [SVG Logos](https://github.com/gilbarbara/logos) `telegram` | CC0 1.0 |
| `slack.svg` | Slack | SVG Logos `slack-icon` | CC0 1.0 |
| `dingtalk.svg` | DingTalk | [UIW Icons](https://github.com/uiwjs/icons) `dingding` | MIT (below) |
| `feishu.svg` | Feishu / Lark | [Semi Design](https://github.com/DouyinFE/semi-design) `feishu_logo` (ByteDance's own icon set), each piece filled with the Feishu brand colors `#00d6b9`, `#163c9a` and `#3370ff` | MIT (below) |
| `phone.svg` | the iPhone app | drawn for AskHuman | the project license |
| `window.svg` | the local popup window | drawn for AskHuman | the project license |

The sources are otherwise kept as published (only the `1em` size of the SVG Logos files was
dropped, and the black fill of the one-colour marks was normalized so the tile can paint it through
a mask). SVG Logos is dedicated to the public domain (CC0 1.0).

### Semi Design

MIT License

Copyright (c) 2021 DouyinFE

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

### UIW Icons

MIT License

Copyright (c) 2019 uiw

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
