//! 文本类附件 → docx（OOXML + zip）生成。
//!
//! 背景见 `docs/plans/dingtalk-attachment-preview.md`：钉钉 `sampleFile` 的 fileType 白名单
//! 不含 md/txt/代码，但含 docx；docx 可被钉钉原生预览，中文由钉钉端渲染（**不内置字体**）。
//!
//! 字体关键事实（实测）：钉钉 docx 渲染器**只认命名段落样式里的字体、忽略 docDefaults**；
//! 故所有字体写进命名样式（英文 Arial、中文 SimHei、代码 Courier New），并让标题等样式
//! `basedOn=Normal` 继承。样式比例/配色套用 GitHub Markdown 风格。
//!
//! 两种渲染模式：
//! - Markdown：`.md/.markdown`，用 `pulldown-cmark` 解析为 OOXML（不额外加文件名标题）。
//! - PlainCode：非 md（代码/txt/json/log…），顶部文件名 H1 + 一个等宽代码块（不做语法高亮）。
//!
//! 列表用“手动标记”（`•` / `1.`）+ 缩进实现，避免 numbering.xml 的编号续接问题，视觉等价。

use std::io::Write;

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use zip::write::SimpleFileOptions;

// ===== 固定 OOXML part =====

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
</Types>"#;

const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;

const DOC_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>"#;

// 命名样式：字体/字号/行距/配色（实测锁定值，详见 docs/plans）。
//
// 注意：标题**不使用命名样式**（钉钉 docx 渲染器会把命名标题样式/带 keepNext 段落的字号
// 抹平成同一档），改为「普通段落 + run 上直接写 加粗+字号」实现（见 heading_* 助手）。
const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/>
<w:pPr><w:spacing w:after="240" w:line="360" w:lineRule="auto"/></w:pPr>
<w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:eastAsia="SimHei" w:cs="Arial"/>
<w:color w:val="1F2328"/><w:sz w:val="24"/><w:szCs w:val="24"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:basedOn w:val="Normal"/>
<w:pPr><w:spacing w:after="60"/><w:ind w:left="640" w:hanging="360"/></w:pPr></w:style>
<w:style w:type="paragraph" w:styleId="Code"><w:name w:val="Code"/><w:basedOn w:val="Normal"/>
<w:pPr><w:spacing w:before="0" w:after="0" w:line="288" w:lineRule="auto"/></w:pPr>
<w:rPr><w:rFonts w:ascii="Courier New" w:hAnsi="Courier New" w:eastAsia="SimHei" w:cs="Courier New"/>
<w:b/><w:bCs/><w:color w:val="1A1A1A"/><w:sz w:val="21"/><w:szCs w:val="21"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Quote"><w:name w:val="Quote"/><w:basedOn w:val="Normal"/>
<w:pPr><w:spacing w:before="60" w:after="200"/><w:ind w:left="480"/>
<w:pBdr><w:left w:val="single" w:sz="24" w:space="10" w:color="D0D7DE"/></w:pBdr></w:pPr>
<w:rPr><w:color w:val="656D76"/></w:rPr></w:style>
</w:styles>"#;

const SECT_PR: &str = r#"<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1134" w:right="1134" w:bottom="1134" w:left="1134" w:header="720" w:footer="720"/></w:sectPr>"#;

/// 正文区可用宽度（twips，A4 减页边距），表格/代码框统一用。
const CONTENT_WIDTH: i32 = 9300;

// ===== 公共入口 =====

/// 把 markdown 文本渲染为 docx 字节（Markdown 模式）。
pub fn build_markdown_docx(content: &str) -> std::io::Result<Vec<u8>> {
    let body = markdown_to_body(content);
    package(&body)
}

/// 把任意文本作为“文件名标题 + 等宽代码块”渲染为 docx 字节（PlainCode 模式）。
pub fn build_plaincode_docx(file_name: &str, content: &str) -> std::io::Result<Vec<u8>> {
    let mut body = String::new();
    body.push_str(&heading_para(1, file_name));
    body.push_str(&code_table(content));
    package(&body)
}

/// 打包已拼好的 `w:body` 内部 OOXML（不含 body/sect 外壳）。供 diff 等需要逐 run 上色的导出使用。
pub fn build_raw_docx(body_inner_xml: &str) -> std::io::Result<Vec<u8>> {
    package(body_inner_xml)
}

/// 标题段落（供外部 diff/transcript 导出复用）。
pub fn ooxml_heading(level: u8, text: &str) -> String {
    heading_para(level, text)
}

/// 普通段落（正文）。
pub fn ooxml_para(text: &str) -> String {
    format!(
        r#"<w:p><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
        esc(text)
    )
}

/// 等宽一行：可选底色 `fill`（hex 无 #，如 `E6FFEC`）与文字色 `color`（hex 无 #）。
pub fn ooxml_mono_line(text: &str, fill: Option<&str>, color: Option<&str>) -> String {
    let mut rpr = String::from(
        r#"<w:rFonts w:ascii="Courier New" w:hAnsi="Courier New" w:eastAsia="SimHei" w:cs="Courier New"/><w:sz w:val="18"/><w:szCs w:val="18"/>"#,
    );
    if let Some(c) = color {
        rpr.push_str(&format!(r#"<w:color w:val="{c}"/>"#));
    }
    let shd = fill
        .map(|f| format!(r#"<w:shd w:val="clear" w:color="auto" w:fill="{f}"/>"#))
        .unwrap_or_default();
    format!(
        r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/>{shd}</w:pPr><w:r><w:rPr>{rpr}</w:rPr><w:t xml:space="preserve">{t}</w:t></w:r></w:p>"#,
        shd = shd,
        rpr = rpr,
        t = esc(text),
    )
}

// ===== zip 打包 =====

fn package(body_xml: &str) -> std::io::Result<Vec<u8>> {
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}{sect}</w:body></w:document>"#,
        body = body_xml,
        sect = SECT_PR,
    );

    let mut buf = Vec::new();
    {
        let cursor = std::io::Cursor::new(&mut buf);
        let mut zip = zip::ZipWriter::new(cursor);
        let opts =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let mut put = |name: &str, data: &str| -> std::io::Result<()> {
            zip.start_file(name, opts)?;
            zip.write_all(data.as_bytes())?;
            Ok(())
        };
        put("[Content_Types].xml", CONTENT_TYPES)?;
        put("_rels/.rels", ROOT_RELS)?;
        put("word/_rels/document.xml.rels", DOC_RELS)?;
        put("word/styles.xml", STYLES)?;
        put("word/document.xml", &document)?;
        zip.finish()?;
    }
    Ok(buf)
}

// ===== XML 助手 =====

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            _ => o.push(c),
        }
    }
    o
}

/// 普通文字 run，按当前粗/斜样式。
fn text_run(text: &str, bold: bool, italic: bool) -> String {
    let mut rpr = String::new();
    if bold {
        rpr.push_str("<w:b/>");
    }
    if italic {
        rpr.push_str("<w:i/>");
    }
    let rpr = if rpr.is_empty() {
        String::new()
    } else {
        format!("<w:rPr>{}</w:rPr>", rpr)
    };
    format!(
        r#"<w:r>{}<w:t xml:space="preserve">{}</w:t></w:r>"#,
        rpr,
        esc(text)
    )
}

/// 行内代码 run（等宽 + 浅灰底）。
fn inline_code_run(text: &str) -> String {
    format!(
        r#"<w:r><w:rPr><w:rFonts w:ascii="Courier New" w:hAnsi="Courier New" w:eastAsia="SimHei"/><w:shd w:val="clear" w:color="auto" w:fill="EFF1F3"/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r>"#,
        esc(text)
    )
}

/// 标题字号（half-point）。钉钉 docx 渲染器对字号不可靠：命名样式/带 keepNext 的标题会被
/// 抹平、相近字号会倒置（如 sz30>sz36、sz32>sz40），且 sz≈36 处有"塌缩"断点。实测可单调
/// 渲染的安全取值：H1=56(28pt) 用高位大字，H2=32(16pt)、H3+=28(14pt) 落在干净单调带内，
/// 正文=24(12pt)。H4–H6 退化到 H3。
fn heading_sz(level: u8) -> u32 {
    match level {
        1 => 56,
        2 => 32,
        _ => 28,
    }
}

/// 标题段落属性：段前/后距 + H1/H2 下边框（GitHub 风格浅灰下划线）。
/// 不用 keepNext / 不用命名样式，避免触发钉钉字号抹平。
fn heading_ppr(level: u8) -> String {
    let (before, after, border) = match level {
        1 => (640, 240, true),
        2 => (600, 200, true),
        _ => (480, 160, false),
    };
    let bdr = if border {
        r#"<w:pBdr><w:bottom w:val="single" w:sz="6" w:space="6" w:color="D8DEE4"/></w:pBdr>"#
    } else {
        ""
    };
    format!(
        r#"<w:pPr><w:spacing w:before="{before}" w:after="{after}"/>{bdr}</w:pPr>"#,
        before = before,
        after = after,
        bdr = bdr,
    )
}

/// 标题文字 run：加粗 + 直接写字号（钉钉只在 run 直接字号、且非"标题样段落"时才生效）。
fn heading_run(level: u8, text: &str) -> String {
    let sz = heading_sz(level);
    format!(
        r#"<w:r><w:rPr><w:b/><w:sz w:val="{sz}"/><w:szCs w:val="{sz}"/></w:rPr><w:t xml:space="preserve">{t}</w:t></w:r>"#,
        sz = sz,
        t = esc(text),
    )
}

/// 独立标题段落（PlainCode 模式用）。
fn heading_para(level: u8, text: &str) -> String {
    format!(
        "<w:p>{}{}</w:p>",
        heading_ppr(level),
        heading_run(level, text)
    )
}

/// 等宽代码框（单元格表格，浅灰底、无边框、不高亮）。
fn code_table(code: &str) -> String {
    let code = code.strip_suffix('\n').unwrap_or(code);
    let mut paras = String::new();
    for line in code.split('\n') {
        if line.is_empty() {
            paras.push_str(r#"<w:p><w:pPr><w:pStyle w:val="Code"/></w:pPr></w:p>"#);
        } else {
            paras.push_str(&format!(
                r#"<w:p><w:pPr><w:pStyle w:val="Code"/></w:pPr><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
                esc(line)
            ));
        }
    }
    format!(
        r#"<w:tbl><w:tblPr><w:tblW w:w="{w}" w:type="dxa"/><w:tblCellMar><w:left w:w="200" w:type="dxa"/><w:right w:w="200" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tr><w:tc><w:tcPr><w:tcW w:w="{w}" w:type="dxa"/><w:shd w:val="clear" w:color="auto" w:fill="F7F8FA"/><w:tcMar><w:top w:w="120" w:type="dxa"/><w:left w:w="200" w:type="dxa"/><w:bottom w:w="120" w:type="dxa"/><w:right w:w="200" w:type="dxa"/></w:tcMar></w:tcPr>{paras}</w:tc></w:tr></w:tbl><w:p/>"#,
        w = CONTENT_WIDTH,
        paras = paras,
    )
}

/// 数据表格：细边框 + 表头加粗 + 偶数行斑马底色。
fn data_table(rows: &[TableRow]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let ncols = rows.iter().map(|r| r.cells.len()).max().unwrap_or(1).max(1) as i32;
    let colw = CONTENT_WIDTH / ncols;
    let borders = ["top", "left", "bottom", "right", "insideH", "insideV"]
        .iter()
        .map(|s| {
            format!(
                r#"<w:{s} w:val="single" w:sz="4" w:space="0" w:color="D0D7DE"/>"#,
                s = s
            )
        })
        .collect::<String>();

    let mut trs = String::new();
    for (i, row) in rows.iter().enumerate() {
        let header = i == 0;
        let zebra = i % 2 == 1;
        let mut tcs = String::new();
        for c in 0..ncols as usize {
            let text = row.cells.get(c).map(|s| s.as_str()).unwrap_or("");
            let shd = if zebra {
                r#"<w:shd w:val="clear" w:color="auto" w:fill="F6F8FA"/>"#
            } else {
                ""
            };
            tcs.push_str(&format!(
                r#"<w:tc><w:tcPr><w:tcW w:w="{cw}" w:type="dxa"/>{shd}<w:vAlign w:val="center"/></w:tcPr><w:p>{run}</w:p></w:tc>"#,
                cw = colw,
                shd = shd,
                run = text_run(text, header, false),
            ));
        }
        let trpr = if header {
            "<w:trPr><w:tblHeader/></w:trPr>"
        } else {
            ""
        };
        trs.push_str(&format!("<w:tr>{}{}</w:tr>", trpr, tcs));
    }

    format!(
        r#"<w:tbl><w:tblPr><w:tblW w:w="{w}" w:type="dxa"/><w:tblBorders>{borders}</w:tblBorders><w:tblCellMar><w:top w:w="80" w:type="dxa"/><w:left w:w="160" w:type="dxa"/><w:bottom w:w="80" w:type="dxa"/><w:right w:w="160" w:type="dxa"/></w:tblCellMar></w:tblPr>{trs}</w:tbl><w:p/>"#,
        w = CONTENT_WIDTH,
        borders = borders,
        trs = trs,
    )
}

// ===== Markdown → OOXML body =====

struct TableRow {
    cells: Vec<String>,
}

/// 当前正在累积的段落。
struct CurPara {
    ppr: String,
    runs: String,
}

struct ListCtx {
    ordered: bool,
    next: u64,
}

#[derive(Default)]
struct Builder {
    out: String,
    cur: Option<CurPara>,
    bold: u32,
    italic: u32,
    quote_depth: u32,
    /// 当前所在标题级别（None=非标题）。标题内文字用 heading_run（加粗+直接字号）。
    heading: Option<u8>,
    lists: Vec<ListCtx>,
    // 代码块
    in_code: bool,
    code_buf: String,
    // 表格
    table_rows: Vec<TableRow>,
    cur_row: Option<Vec<String>>,
    cur_cell: Option<String>,
}

impl Builder {
    fn flush_para(&mut self) {
        if let Some(p) = self.cur.take() {
            self.out
                .push_str(&format!("<w:p>{}{}</w:p>", p.ppr, p.runs));
        }
    }

    fn open_para(&mut self, ppr: String) {
        self.flush_para();
        self.cur = Some(CurPara {
            ppr,
            runs: String::new(),
        });
    }

    /// 文字到来时若无打开段落则惰性开一个（正文/引用）。
    fn ensure_para(&mut self) {
        if self.cur.is_none() {
            let ppr = if self.quote_depth > 0 {
                r#"<w:pPr><w:pStyle w:val="Quote"/></w:pPr>"#.to_string()
            } else {
                String::new()
            };
            self.cur = Some(CurPara {
                ppr,
                runs: String::new(),
            });
        }
    }

    fn push_run(&mut self, run: String) {
        self.ensure_para();
        if let Some(p) = self.cur.as_mut() {
            p.runs.push_str(&run);
        }
    }

    fn list_item_ppr(&self) -> (String, String) {
        let depth = self.lists.len().saturating_sub(1) as i32;
        let left = 640 + depth * 420;
        let ppr = format!(
            r#"<w:pPr><w:pStyle w:val="ListParagraph"/><w:ind w:left="{left}" w:hanging="360"/></w:pPr>"#,
            left = left
        );
        let marker = match self.lists.last() {
            Some(ctx) if ctx.ordered => format!("{}. ", ctx.next),
            _ => "• ".to_string(),
        };
        (ppr, marker)
    }

    fn handle(&mut self, ev: Event) {
        match ev {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => {
                if self.in_code {
                    self.code_buf.push_str(&t);
                } else if let Some(cell) = self.cur_cell.as_mut() {
                    cell.push_str(&t);
                } else if let Some(level) = self.heading {
                    let run = heading_run(level, &t);
                    self.push_run(run);
                } else {
                    let run = text_run(&t, self.bold > 0, self.italic > 0);
                    self.push_run(run);
                }
            }
            Event::Code(t) => {
                if self.in_code {
                    self.code_buf.push_str(&t);
                } else if let Some(cell) = self.cur_cell.as_mut() {
                    cell.push_str(&t);
                } else if let Some(level) = self.heading {
                    // 标题内的行内代码并入标题样式（加粗+字号），不另作浅灰底。
                    let run = heading_run(level, &t);
                    self.push_run(run);
                } else {
                    let run = inline_code_run(&t);
                    self.push_run(run);
                }
            }
            Event::SoftBreak => {
                if let Some(cell) = self.cur_cell.as_mut() {
                    cell.push(' ');
                } else if !self.in_code {
                    self.push_run(text_run(" ", false, false));
                }
            }
            Event::HardBreak => {
                if !self.in_code && self.cur_cell.is_none() {
                    self.push_run("<w:r><w:br/></w:r>".to_string());
                }
            }
            Event::Rule => {
                self.flush_para();
                self.out.push_str(
                    r#"<w:p><w:pPr><w:pBdr><w:bottom w:val="single" w:sz="6" w:space="6" w:color="D8DEE4"/></w:pBdr></w:pPr></w:p>"#,
                );
            }
            Event::TaskListMarker(checked) => {
                let m = if checked { "[x] " } else { "[ ] " };
                self.push_run(text_run(m, false, false));
            }
            _ => {} // Html / InlineHtml / 脚注 / 数学等：忽略
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                // 列表项内已开段落则沿用；否则开正文/引用段落。
                if self.cur.is_none() {
                    let ppr = if self.quote_depth > 0 {
                        r#"<w:pPr><w:pStyle w:val="Quote"/></w:pPr>"#.to_string()
                    } else {
                        String::new()
                    };
                    self.open_para(ppr);
                }
            }
            Tag::Heading { level, .. } => {
                let lvl = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    _ => 3,
                };
                self.heading = Some(lvl);
                self.open_para(heading_ppr(lvl));
            }
            Tag::BlockQuote(_) => {
                self.flush_para();
                self.quote_depth += 1;
            }
            Tag::CodeBlock(_) => {
                self.flush_para();
                self.in_code = true;
                self.code_buf.clear();
            }
            Tag::List(start) => {
                self.flush_para();
                self.lists.push(ListCtx {
                    ordered: start.is_some(),
                    next: start.unwrap_or(1),
                });
            }
            Tag::Item => {
                let (ppr, marker) = self.list_item_ppr();
                self.open_para(ppr);
                if let Some(p) = self.cur.as_mut() {
                    p.runs.push_str(&text_run(&marker, false, false));
                }
            }
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Table(_) => {
                self.flush_para();
                self.table_rows.clear();
            }
            Tag::TableHead | Tag::TableRow => {
                self.cur_row = Some(Vec::new());
            }
            Tag::TableCell => {
                self.cur_cell = Some(String::new());
            }
            // 链接/图片：仅保留内部文字（不渲染超链接 / 不嵌图片）。
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush_para(),
            TagEnd::Heading(_) => {
                self.flush_para();
                self.heading = None;
            }
            TagEnd::BlockQuote(_) => {
                self.flush_para();
                self.quote_depth = self.quote_depth.saturating_sub(1);
            }
            TagEnd::CodeBlock => {
                let code = std::mem::take(&mut self.code_buf);
                self.out.push_str(&code_table(&code));
                self.in_code = false;
            }
            TagEnd::List(_) => {
                self.lists.pop();
            }
            TagEnd::Item => {
                self.flush_para();
                if let Some(ctx) = self.lists.last_mut() {
                    if ctx.ordered {
                        ctx.next += 1;
                    }
                }
            }
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Table => {
                let rows = std::mem::take(&mut self.table_rows);
                self.out.push_str(&data_table(&rows));
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                if let Some(cells) = self.cur_row.take() {
                    self.table_rows.push(TableRow { cells });
                }
            }
            TagEnd::TableCell => {
                if let Some(cell) = self.cur_cell.take() {
                    if let Some(row) = self.cur_row.as_mut() {
                        row.push(cell);
                    }
                }
            }
            _ => {}
        }
    }
}

fn markdown_to_body(content: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(content, opts);

    let mut b = Builder::default();
    for ev in parser {
        b.handle(ev);
    }
    b.flush_para();
    b.out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unzip_names(bytes: &[u8]) -> Vec<String> {
        let reader = std::io::Cursor::new(bytes);
        let mut zip = zip::ZipArchive::new(reader).unwrap();
        (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().unwrap().into_owned())
            .collect()
    }

    #[test]
    fn markdown_docx_has_required_parts() {
        let bytes = build_markdown_docx("# 标题\n\n正文 **粗体**\n\n- a\n- b\n").unwrap();
        let names = unzip_names(&bytes);
        for need in [
            "[Content_Types].xml",
            "_rels/.rels",
            "word/styles.xml",
            "word/document.xml",
        ] {
            assert!(names.iter().any(|n| n == need), "missing part: {need}");
        }
    }

    #[test]
    fn docx_parts_decompress_as_valid_xml_and_preserve_unicode() {
        use std::io::Read;

        let bytes =
            build_markdown_docx("# 中文标题\n\n正文 & emoji 🐟\n\n```text\n<&> 中文代码\n```\n")
                .unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut document_text = None;
        for index in 0..archive.len() {
            let mut part = archive.by_index(index).unwrap();
            assert_eq!(part.compression(), zip::CompressionMethod::Deflated);
            let mut xml = String::new();
            part.read_to_string(&mut xml).unwrap();
            let parsed = roxmltree::Document::parse(&xml).unwrap();
            if part.name().unwrap() == "word/document.xml" {
                document_text = Some(
                    parsed
                        .descendants()
                        .filter(|node| {
                            node.has_tag_name((
                                "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
                                "t",
                            ))
                        })
                        .filter_map(|node| node.text())
                        .collect::<String>(),
                );
            }
        }
        let text = document_text.expect("DOCX must contain the main document part");
        for expected in ["中文标题", "正文 & emoji 🐟", "<&> 中文代码"] {
            assert!(text.contains(expected), "missing document text: {expected}");
        }
    }

    #[test]
    fn plaincode_docx_has_title_and_code() {
        let bytes = build_plaincode_docx("demo.rs", "fn main() {}\n").unwrap();
        let names = unzip_names(&bytes);
        assert!(names.iter().any(|n| n == "word/document.xml"));
        assert!(bytes.len() > 200);
    }

    #[test]
    fn table_and_code_render() {
        let md = "| a | b |\n| --- | --- |\n| 1 | 2 |\n\n```rust\nlet x=1;\n```\n";
        let bytes = build_markdown_docx(md).unwrap();
        assert!(bytes.len() > 300);
    }

    /// 手动检视用：`cargo test emit_manual_samples -- --ignored`，再打开 /tmp 下两个 docx。
    #[test]
    #[ignore]
    fn emit_manual_samples() {
        let md = "# 钉钉文本附件 · Rust 生成验证\n\n这是一段中文正文，含 **粗体**、*斜体* 与 `行内代码`，中英混排 mixed English 123。\n\n## 1. 章节标题（H2）\n这是 H2 章节下的正文，用于核对 H1 / H2 / H3 字号层级。\n\n### 1.1 子节标题（H3）\n这是 H3 子节正文，应比 H2 小、比正文大。\n\n## 2. 无序列表\n- 短文件：贴成 Markdown 消息\n- 长文件：转 docx 发送\n  - 嵌套项一\n  - 嵌套项二\n\n## 3. 有序列表\n1. 读取并判断字符数\n2. 超阈值转 docx\n3. 上传并发送\n\n## 4. 表格\n| 方式 | 表现 | 结论 |\n| --- | --- | --- |\n| docx | 原生预览 | 可用 |\n| md | 打不开 | 不可用 |\n\n## 5. 代码块\n```rust\nfn render(src: &str) -> Vec<u8> {\n    // 中文注释：转 OOXML 再打包\n    build(markdown_to_ooxml(src))\n}\n```\n\n## 6. 引用\n> 关键认知：docx 在钉钉白名单内，中文由钉钉渲染、无需内置字体。\n\n---\n\n这是分隔线之后的[链接文字](https://example.com)与结尾段落。\n";
        let b1 = build_markdown_docx(md).unwrap();
        std::fs::write("/tmp/ha-rust-md.docx", &b1).unwrap();

        let code = "fn main() {\n    // 顶部是文件名标题，下面整块等宽\n    let xs = vec![1, 2, 3];\n    for x in xs {\n        println!(\"{}\", x);\n    }\n}\n";
        let b2 = build_plaincode_docx("main.rs", code).unwrap();
        std::fs::write("/tmp/ha-rust-code.docx", &b2).unwrap();
        eprintln!(
            "wrote /tmp/ha-rust-md.docx ({} B), /tmp/ha-rust-code.docx ({} B)",
            b1.len(),
            b2.len()
        );
    }
}
