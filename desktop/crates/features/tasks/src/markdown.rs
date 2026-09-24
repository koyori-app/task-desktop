//! 説明文の Markdown 表示。コードブロックだけは行番号付きのエディタ（読み取り専用）で出す。
//! gpui-kit の TextView はコードブロックに色は付くが行番号を出せないため。

use gpui_kit::component::Theme;
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::text::TextView;
use gpui_kit::*;

/// 説明文を分けたもの。
#[derive(Debug, Clone, PartialEq)]
pub enum Segment {
    Text(String),
    Code { lang: Option<String>, code: String },
}

/// 行頭（3 文字までの字下げ）のフェンス ``` / ~~~ でコードブロックを切り出す。
/// 閉じていないフェンスは末尾までをコードとする（CommonMark と同じ）。
pub fn split_code_blocks(src: &str) -> Vec<Segment> {
    let mut segments = vec![];
    let mut text = String::new();
    let mut lines = src.split_inclusive('\n');
    while let Some(line) = lines.next() {
        let Some((fence, len, info)) = open_fence(line) else {
            text.push_str(line);
            continue;
        };
        let mut code = String::new();
        for line in lines.by_ref() {
            if is_close_fence(line, fence, len) {
                break;
            }
            code.push_str(line);
        }
        if !text.trim().is_empty() {
            segments.push(Segment::Text(std::mem::take(&mut text)));
        }
        text.clear();
        if code.ends_with('\n') {
            code.pop();
        }
        let lang = info.split_whitespace().next().map(str::to_string);
        segments.push(Segment::Code { lang, code });
    }
    if !text.trim().is_empty() {
        segments.push(Segment::Text(text));
    }
    segments
}

fn fence_start(line: &str) -> Option<(char, usize, &str)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let fence = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let len = trimmed.chars().take_while(|c| *c == fence).count();
    (len >= 3).then(|| (fence, len, trimmed[len..].trim()))
}

fn open_fence(line: &str) -> Option<(char, usize, &str)> {
    let (fence, len, info) = fence_start(line)?;
    // バッククォートのフェンスは情報文字列にバッククォートを含められない。
    (fence != '`' || !info.contains('`')).then_some((fence, len, info))
}

fn is_close_fence(line: &str, fence: char, open_len: usize) -> bool {
    fence_start(line).is_some_and(|(f, len, rest)| f == fence && len >= open_len && rest.is_empty())
}

/// 説明文の表示。本文が変わった時だけエディタを作り直す。
pub struct MarkdownBlocks {
    source: Option<String>,
    parts: Vec<Part>,
}

enum Part {
    Text(SharedString),
    Code {
        editor: Entity<EditorState>,
        lines: usize,
    },
}

impl MarkdownBlocks {
    pub fn new() -> Self {
        Self {
            source: None,
            parts: vec![],
        }
    }

    pub fn sync(&mut self, source: &str, window: &mut Window, cx: &mut App) {
        if self.source.as_deref() == Some(source) {
            return;
        }
        self.source = Some(source.to_string());
        self.parts = split_code_blocks(source)
            .into_iter()
            .map(|segment| match segment {
                Segment::Text(text) => Part::Text(text.into()),
                Segment::Code { lang, code } => {
                    let lines = code.lines().count().max(1);
                    let editor = cx.new(|cx| {
                        EditorState::new(window, cx)
                            .language(lang.unwrap_or_else(|| "text".into()))
                            .soft_wrap(false)
                            .folding(false)
                            .searchable(false)
                            // 表示だけなので、最後の行より下に空きを作らない（縦にスクロールさせない）。
                            .scroll_beyond_last_line(Some(0))
                            .default_value(code)
                    });
                    Part::Code { editor, lines }
                }
            })
            .collect();
    }

    pub fn render(&self, id: &str, cx: &App) -> impl IntoElement {
        // エディタは内容の高さに合わせる（行の高さはフォントの 1.5 倍）。
        let line_height = Theme::global(cx).mono_font_size * 1.5;
        div()
            .flex()
            .flex_col()
            .gap_3()
            .w_full()
            .children(self.parts.iter().enumerate().map(|(ix, part)| {
                match part {
                    Part::Text(text) => {
                        TextView::markdown(SharedString::from(format!("{id}-{ix}")), text.clone())
                            .w_full()
                            .into_any_element()
                    }
                    Part::Code { editor, lines } => Editor::new(editor)
                        .readonly(true)
                        .h(line_height * *lines as f32 + px(CODE_PADDING))
                        .into_any_element(),
                }
            }))
    }
}

/// エディタの上下の余白と、横スクロールバーの分。
const CODE_PADDING: f32 = 28.;

#[cfg(test)]
mod tests {
    use super::{Segment, split_code_blocks};

    fn code(lang: Option<&str>, code: &str) -> Segment {
        Segment::Code {
            lang: lang.map(str::to_string),
            code: code.to_string(),
        }
    }

    #[test]
    fn splits_fenced_code_from_text() {
        let src = "前\n\n```rust\nfn main() {}\n```\n\n後\n";
        assert_eq!(
            split_code_blocks(src),
            vec![
                Segment::Text("前\n\n".into()),
                code(Some("rust"), "fn main() {}"),
                Segment::Text("\n後\n".into()),
            ]
        );
    }

    #[test]
    fn handles_tilde_longer_and_unclosed_fences() {
        assert_eq!(split_code_blocks("~~~\na\n~~~"), vec![code(None, "a")]);
        // 開きより短い閉じは本文として扱う。
        assert_eq!(
            split_code_blocks("````md\n```\nx\n````\n"),
            vec![code(Some("md"), "```\nx")]
        );
        assert_eq!(split_code_blocks("```\nopen"), vec![code(None, "open")]);
    }

    #[test]
    fn ignores_inline_and_indented_backticks() {
        let src = "インラインの `code` と ``x``\n    ```\n";
        assert_eq!(split_code_blocks(src), vec![Segment::Text(src.into())]);
        assert_eq!(
            split_code_blocks("``` a`b\n"),
            vec![Segment::Text("``` a`b\n".into())]
        );
    }
}
