#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct F1Chunk {
    pub(super) text: String,
    pub(super) start: usize,
    pub(super) heading_path: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DocBlock {
    start: usize,
    end: usize,
    heading_path: Vec<String>,
}

pub(super) fn f1_recursive_chunks(text: &str, max: usize) -> Vec<F1Chunk> {
    let max = max.max(1);
    let blocks = doc_blocks(text);
    let mut chunks = Vec::new();
    for block in blocks {
        if block.end <= block.start {
            continue;
        }
        if block.end - block.start <= max {
            chunks.push(chunk_for_range(
                text,
                block.start,
                block.end,
                block.heading_path,
            ));
        } else {
            chunks.extend(fixed_range(
                text,
                block.start,
                block.end,
                max,
                block.heading_path,
            ));
        }
    }
    if chunks.is_empty() && !text.is_empty() {
        chunks.extend(fixed_range(text, 0, text.len(), max, Vec::new()));
    }
    chunks
}

pub(super) fn line_for_offset(text: &str, offset: usize, base_line: usize) -> usize {
    base_line
        + text[..offset.min(text.len())]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
}

fn doc_blocks(text: &str) -> Vec<DocBlock> {
    let mut blocks = Vec::new();
    let mut headings: Vec<String> = Vec::new();
    let mut paragraph: Option<(usize, usize, Vec<String>)> = None;

    for (line_start, line_end, line) in line_segments(text) {
        if let Some((level, title)) = heading(line) {
            flush_paragraph(&mut blocks, &mut paragraph);
            while headings.len() >= level {
                headings.pop();
            }
            headings.push(title);
        } else if line.trim().is_empty() {
            flush_paragraph(&mut blocks, &mut paragraph);
        } else if let Some((_, end, _)) = &mut paragraph {
            *end = line_end;
        } else {
            paragraph = Some((line_start, line_end, headings.clone()));
        }
    }

    flush_paragraph(&mut blocks, &mut paragraph);
    blocks
}

fn flush_paragraph(
    blocks: &mut Vec<DocBlock>,
    paragraph: &mut Option<(usize, usize, Vec<String>)>,
) {
    let Some((start, end, heading_path)) = paragraph.take() else {
        return;
    };
    if end > start {
        blocks.push(DocBlock {
            start,
            end,
            heading_path,
        });
    }
}

fn fixed_range(
    text: &str,
    start: usize,
    end: usize,
    max: usize,
    heading_path: Vec<String>,
) -> Vec<F1Chunk> {
    let mut chunks = Vec::new();
    let mut cursor = start;
    while cursor < end {
        let mut next = (cursor + max).min(end);
        while next > cursor && !text.is_char_boundary(next) {
            next -= 1;
        }
        if next == cursor {
            next = end;
        }
        chunks.push(chunk_for_range(text, cursor, next, heading_path.clone()));
        cursor = next;
    }
    chunks
}

fn chunk_for_range(text: &str, start: usize, end: usize, heading_path: Vec<String>) -> F1Chunk {
    F1Chunk {
        text: text[start..end].trim().to_owned(),
        start,
        heading_path,
    }
}

fn line_segments(text: &str) -> impl Iterator<Item = (usize, usize, &str)> {
    let mut cursor = 0;
    std::iter::from_fn(move || {
        if cursor >= text.len() {
            return None;
        }
        let start = cursor;
        let rest = &text[start..];
        let (end, next) = if let Some(relative) = rest.find('\n') {
            let line_end = start + relative;
            (line_end, line_end + 1)
        } else {
            (text.len(), text.len())
        };
        cursor = next;
        Some((start, end, &text[start..end]))
    })
}

fn heading(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim_start();
    let hashes = trimmed.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let title = trimmed[hashes..].trim();
    if title.is_empty() {
        None
    } else {
        Some((hashes, title.to_owned()))
    }
}
