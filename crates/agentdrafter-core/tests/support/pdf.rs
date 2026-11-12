//! A small reader of the PDFs a plot writes, for tests: the page's size,
//! its text as a reader extracts it, through each font's `ToUnicode` map,
//! and the pens its strokes are drawn with. It shares no code with the
//! writer, so a test that reads a PDF with it checks what the file holds,
//! not what the writer meant. The core's tests and the command line's both
//! include this file.

#![allow(dead_code)]

use std::collections::HashMap;

/// A PDF's objects, by number, each its dictionary's text and its stream
/// unpacked.
pub struct Pdf {
    pub bytes: Vec<u8>,
    objects: HashMap<u32, (String, Option<Vec<u8>>)>,
}

/// One token of a content stream.
#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f64),
    Name(String),
    Text(Vec<u8>),
    Open,
    Close,
    Operator(String),
}

/// A line of text the PDF holds, with where it starts on the page in
/// points and the frame it is set in.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLine {
    pub text: String,
    pub matrix: [f64; 6],
}

/// A stroke the page draws: its colour, red, green and blue from 0 to 1,
/// and its width in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pen {
    pub rgb: [f64; 3],
    pub width: f64,
}

impl Pdf {
    pub fn read(bytes: Vec<u8>) -> Pdf {
        assert!(bytes.starts_with(b"%PDF-"), "not a PDF");
        let mut objects = HashMap::new();
        let mut at = 0;
        while let Some(found) = find(&bytes[at..], b" 0 obj") {
            let start = at + found;
            // The object's number, just before.
            let digits = bytes[..start]
                .iter()
                .rev()
                .take_while(|b| b.is_ascii_digit())
                .count();
            let number: u32 = std::str::from_utf8(&bytes[start - digits..start])
                .unwrap()
                .parse()
                .unwrap();
            let body_start = start + b" 0 obj".len();
            let end = body_start + find(&bytes[body_start..], b"endobj").unwrap();
            let body = &bytes[body_start..end];
            let (dict, stream) = match find(body, b"stream\n") {
                Some(s) => {
                    let data_start = s + b"stream\n".len();
                    let data_end = find(body, b"\nendstream").unwrap();
                    let dict = String::from_utf8_lossy(&body[..s]).into_owned();
                    let raw = &body[data_start..data_end];
                    let data = if dict.contains("/FlateDecode") {
                        miniz_oxide::inflate::decompress_to_vec_zlib(raw).unwrap()
                    } else {
                        raw.to_vec()
                    };
                    (dict, Some(data))
                }
                None => (String::from_utf8_lossy(body).into_owned(), None),
            };
            objects.insert(number, (dict, stream));
            at = end;
        }
        Pdf { bytes, objects }
    }

    fn object_with(&self, key: &str) -> Vec<(u32, &String)> {
        let mut found: Vec<(u32, &String)> = self
            .objects
            .iter()
            .filter(|(_, (dict, _))| dict.contains(key))
            .map(|(n, (dict, _))| (*n, dict))
            .collect();
        found.sort_by_key(|(n, _)| *n);
        found
    }

    /// The numbers after `key` in the dictionary `dict`, up to the next
    /// key or the end of an array.
    fn numbers_after(dict: &str, key: &str) -> Vec<f64> {
        let at = dict
            .find(key)
            .unwrap_or_else(|| panic!("no {key} in {dict}"))
            + key.len();
        dict[at..]
            .trim_start()
            .trim_start_matches('[')
            .split([']', '/', '>'])
            .next()
            .unwrap()
            .split_whitespace()
            .map_while(|n| n.parse().ok())
            .collect()
    }

    fn reference_after(dict: &str, key: &str) -> u32 {
        let numbers = Pdf::numbers_after(dict, key);
        numbers[0] as u32
    }

    /// The one page's box, `[left, bottom, right, top]` in points.
    pub fn media_box(&self) -> [f64; 4] {
        let pages = self.object_with("/MediaBox");
        assert_eq!(pages.len(), 1, "one page");
        let numbers = Pdf::numbers_after(pages[0].1, "/MediaBox");
        [numbers[0], numbers[1], numbers[2], numbers[3]]
    }

    /// The text of the document information's entry `key`, such as
    /// `/Producer`, if it is a literal string.
    pub fn info(&self, key: &str) -> Option<String> {
        let (_, dict) = self.object_with("/Producer").into_iter().next()?;
        let at = dict.find(&format!("{key} ("))? + key.len() + 1;
        let tokens = tokens(&dict.as_bytes()[at..]);
        match tokens.first() {
            Some(Token::Text(bytes)) => Some(String::from_utf8_lossy(bytes).into_owned()),
            _ => None,
        }
    }

    /// The page's content, unpacked.
    fn content(&self) -> &[u8] {
        let page = self.object_with("/MediaBox")[0].1;
        let number = Pdf::reference_after(page, "/Contents");
        self.objects[&number].1.as_deref().unwrap()
    }

    /// Each font the page names, by its resource name, with its map from
    /// codes to the text they stand for.
    fn fonts(&self) -> HashMap<String, HashMap<u8, String>> {
        let page = self.object_with("/MediaBox")[0].1;
        let mut fonts = HashMap::new();
        let Some(at) = page.find("/Font") else {
            return fonts;
        };
        let rest = &page[at + "/Font".len()..];
        let rest = &rest[..rest.find(">>").unwrap()];
        let parts: Vec<&str> = rest
            .trim_start()
            .trim_start_matches("<<")
            .split_whitespace()
            .collect();
        for entry in parts.chunks(4) {
            let name = entry[0].trim_start_matches('/').to_owned();
            let number: u32 = entry[1].parse().unwrap();
            let font = &self.objects[&number].0;
            let map = Pdf::reference_after(font, "/ToUnicode");
            let cmap =
                String::from_utf8_lossy(self.objects[&map].1.as_deref().unwrap()).into_owned();
            fonts.insert(name, unicode_map(&cmap));
        }
        fonts
    }

    /// Each line of text the page holds, as a reader extracts it: each
    /// string through its font's `ToUnicode` map, a line for each text
    /// matrix set.
    pub fn text(&self) -> Vec<TextLine> {
        let fonts = self.fonts();
        let mut lines: Vec<TextLine> = Vec::new();
        let mut font: Option<&HashMap<u8, String>> = None;
        let mut stack: Vec<Token> = Vec::new();
        let mut in_array = false;
        let mut array: Vec<Token> = Vec::new();
        for token in tokens(self.content()) {
            match token {
                Token::Open => {
                    in_array = true;
                    array.clear();
                }
                Token::Close => {
                    in_array = false;
                    stack.push(Token::Text(Vec::new()));
                }
                Token::Operator(op) => {
                    match op.as_str() {
                        "Tf" => {
                            let Some(Token::Name(name)) = stack.first() else {
                                panic!("Tf without a font: {stack:?}");
                            };
                            font = fonts.get(name);
                        }
                        "Tm" => {
                            let m: Vec<f64> = stack
                                .iter()
                                .map(|t| match t {
                                    Token::Number(n) => *n,
                                    _ => panic!("{t:?}"),
                                })
                                .collect();
                            lines.push(TextLine {
                                text: String::new(),
                                matrix: [m[0], m[1], m[2], m[3], m[4], m[5]],
                            });
                        }
                        "Tj" | "TJ" => {
                            let strings: Vec<Vec<u8>> = if op == "Tj" {
                                stack
                                    .iter()
                                    .filter_map(|t| match t {
                                        Token::Text(b) => Some(b.clone()),
                                        _ => None,
                                    })
                                    .collect()
                            } else {
                                array
                                    .iter()
                                    .filter_map(|t| match t {
                                        Token::Text(b) => Some(b.clone()),
                                        _ => None,
                                    })
                                    .collect()
                            };
                            let map = font.expect("text shown without a font");
                            let line = lines.last_mut().expect("text shown before a matrix");
                            for code in strings.concat() {
                                line.text.push_str(
                                    map.get(&code)
                                        .unwrap_or_else(|| panic!("code {code} is not mapped")),
                                );
                            }
                        }
                        _ => {}
                    }
                    stack.clear();
                }
                other if in_array => array.push(other),
                other => stack.push(other),
            }
        }
        lines
    }

    /// The pen of each stroke the page paints, in order.
    pub fn strokes(&self) -> Vec<Pen> {
        let mut pens = Vec::new();
        let mut rgb = [0.0; 3];
        let mut width = 1.0;
        let mut stack: Vec<f64> = Vec::new();
        let mut in_text = false;
        for token in tokens(self.content()) {
            match token {
                Token::Number(n) => stack.push(n),
                Token::Operator(op) => {
                    match op.as_str() {
                        "BT" => in_text = true,
                        "ET" => in_text = false,
                        "RG" => rgb = [stack[0], stack[1], stack[2]],
                        "G" => rgb = [stack[0]; 3],
                        "w" => width = stack[0],
                        "S" if !in_text => pens.push(Pen { rgb, width }),
                        _ => {}
                    }
                    stack.clear();
                }
                _ => stack.clear(),
            }
        }
        pens
    }

    /// Every point the page's paths are drawn through, in points, in order.
    pub fn points(&self) -> Vec<[f64; 2]> {
        let mut points = Vec::new();
        let mut stack: Vec<f64> = Vec::new();
        let mut in_text = false;
        for token in tokens(self.content()) {
            match token {
                Token::Number(n) => stack.push(n),
                Token::Operator(op) => {
                    match op.as_str() {
                        "BT" => in_text = true,
                        "ET" => in_text = false,
                        "m" | "l" if !in_text => points.push([stack[0], stack[1]]),
                        _ => {}
                    }
                    stack.clear();
                }
                _ => stack.clear(),
            }
        }
        points
    }

    /// The colour of each fill the page paints, in order.
    pub fn fills(&self) -> Vec<[f64; 3]> {
        let mut fills = Vec::new();
        let mut rgb = [0.0; 3];
        let mut stack: Vec<f64> = Vec::new();
        for token in tokens(self.content()) {
            match token {
                Token::Number(n) => stack.push(n),
                Token::Operator(op) => {
                    match op.as_str() {
                        "rg" => rgb = [stack[0], stack[1], stack[2]],
                        "g" => rgb = [stack[0]; 3],
                        "f" | "f*" => fills.push(rgb),
                        _ => {}
                    }
                    stack.clear();
                }
                _ => stack.clear(),
            }
        }
        fills
    }
}

/// The codes a `ToUnicode` map maps, each to its text.
fn unicode_map(cmap: &str) -> HashMap<u8, String> {
    let mut map = HashMap::new();
    let mut in_chars = false;
    for line in cmap.lines() {
        if line.ends_with("beginbfchar") {
            in_chars = true;
        } else if line == "endbfchar" {
            in_chars = false;
        } else if in_chars {
            let parts: Vec<&str> = line
                .split(['<', '>'])
                .filter(|p| !p.trim().is_empty())
                .collect();
            let code = u8::from_str_radix(parts[0], 16).unwrap();
            let units: Vec<u16> = parts[1]
                .as_bytes()
                .chunks(4)
                .map(|c| u16::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
                .collect();
            map.insert(code, String::from_utf16(&units).unwrap());
        }
    }
    map
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// `bytes`, a PDF with a cross-reference table, such as a plot writes,
/// saved again as `qpdf --object-streams=generate` saves it: every object
/// that is not a stream moved into one object stream, and the table and
/// its trailer replaced by a cross-reference stream, packed with deflate
/// through PNG's Up predictor, whose dictionary is the trailer. The objects
/// keep their numbers.
pub fn with_object_streams(bytes: &[u8]) -> Vec<u8> {
    let start = bytes.windows(9).rposition(|w| w == b"startxref").unwrap();
    let words = |at: usize| {
        String::from_utf8_lossy(&bytes[at..])
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let table: usize = words(start + 9)[0].parse().unwrap();
    let table_words = words(table);
    assert_eq!(table_words[0], "xref", "a cross-reference table");
    let trailer = table_words.iter().position(|w| w == "trailer").unwrap();
    // Where each object in use starts, by its number.
    let first: u32 = table_words[1].parse().unwrap();
    let mut starts: Vec<(usize, u32)> = table_words[3..trailer]
        .chunks(3)
        .zip(first..)
        .filter(|(entry, _)| entry[2] == "n")
        .map(|(entry, number)| (entry[0].parse().unwrap(), number))
        .collect();
    starts.sort();
    let trailer_text = table_words[trailer..].join(" ");
    let reference = |key: &str| Pdf::reference_after(&trailer_text, key);
    let (root, info) = (reference("/Root"), reference("/Info"));
    // Each object's body, between `N 0 obj` and `endobj`, as written.
    let mut objects: Vec<(u32, &[u8])> = Vec::new();
    for (i, &(at, number)) in starts.iter().enumerate() {
        let end = starts.get(i + 1).map_or(table, |&(next, _)| next);
        let whole = &bytes[at..end];
        let heading = format!("{number} 0 obj");
        assert!(whole.starts_with(heading.as_bytes()), "object {number}");
        let body = whole[heading.len()..].trim_ascii_end();
        objects.push((number, body.strip_suffix(b"endobj").unwrap()));
    }
    let size = objects.iter().map(|(n, _)| n + 1).max().unwrap();
    let (stream_number, xref_number) = (size, size + 1);
    // Each object's row: its type, then its place or object stream, then
    // its generation or index.
    let mut rows = vec![(0u8, 0u32, 0u16); xref_number as usize + 1];
    let mut out = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let (streams, plain): (Vec<_>, Vec<_>) = objects
        .iter()
        .partition(|(_, body)| body.trim_ascii_end().ends_with(b"endstream"));
    for &(number, body) in streams {
        rows[number as usize] = (1, out.len() as u32, 0);
        out.extend(format!("{number} 0 obj").as_bytes());
        out.extend(body);
        out.extend(b"endobj\n");
    }
    let mut heading = String::new();
    let mut data = Vec::new();
    for (i, &&(number, body)) in plain.iter().enumerate() {
        heading.push_str(&format!("{number} {} ", data.len()));
        data.extend(body.trim_ascii());
        data.push(b'\n');
        rows[number as usize] = (2, stream_number, i as u16);
    }
    let first = heading.len();
    let mut unpacked = heading.into_bytes();
    unpacked.extend(data);
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&unpacked, 6);
    rows[stream_number as usize] = (1, out.len() as u32, 0);
    out.extend(
        format!(
            "{stream_number} 0 obj\n<< /Type /ObjStm /N {} /First {first} /Filter /FlateDecode \
             /Length {} >>\nstream\n",
            plain.len(),
            packed.len()
        )
        .as_bytes(),
    );
    out.extend(packed);
    out.extend(b"\nendstream\nendobj\n");
    let xref_at = out.len();
    rows[xref_number as usize] = (1, xref_at as u32, 0);
    let mut predicted = Vec::new();
    let mut above = [0u8; 7];
    for (kind, second, third) in rows {
        let mut row = [0u8; 7];
        row[0] = kind;
        row[1..5].copy_from_slice(&second.to_be_bytes());
        row[5..].copy_from_slice(&third.to_be_bytes());
        predicted.push(2);
        predicted.extend(row.iter().zip(above).map(|(b, up)| b.wrapping_sub(up)));
        above = row;
    }
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&predicted, 6);
    out.extend(
        format!(
            "{xref_number} 0 obj\n<< /Type /XRef /Size {} /W [1 4 2] /Root {root} 0 R /Info \
             {info} 0 R /Filter /FlateDecode /DecodeParms << /Columns 7 /Predictor 12 >> \
             /Length {} >>\nstream\n",
            xref_number + 1,
            packed.len()
        )
        .as_bytes(),
    );
    out.extend(packed);
    out.extend(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    out
}

/// The tokens of a content stream.
fn tokens(bytes: &[u8]) -> Vec<Token> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b if b.is_ascii_whitespace() => i += 1,
            b'[' => {
                out.push(Token::Open);
                i += 1;
            }
            b']' => {
                out.push(Token::Close);
                i += 1;
            }
            b'/' => {
                let end = i
                    + 1
                    + bytes[i + 1..]
                        .iter()
                        .position(|c| c.is_ascii_whitespace() || b"/[]()<>".contains(c))
                        .unwrap_or(bytes.len() - i - 1);
                out.push(Token::Name(
                    String::from_utf8_lossy(&bytes[i + 1..end]).into_owned(),
                ));
                i = end;
            }
            b'(' => {
                let mut text = Vec::new();
                let mut depth = 0;
                i += 1;
                loop {
                    let c = bytes[i];
                    i += 1;
                    match c {
                        b'\\' => {
                            let e = bytes[i];
                            i += 1;
                            match e {
                                b'n' => text.push(b'\n'),
                                b'r' => text.push(b'\r'),
                                b't' => text.push(b'\t'),
                                b'b' => text.push(8),
                                b'f' => text.push(12),
                                b'0'..=b'7' => {
                                    let mut v = u32::from(e - b'0');
                                    for _ in 0..2 {
                                        if let Some(&d @ b'0'..=b'7') = bytes.get(i) {
                                            v = v * 8 + u32::from(d - b'0');
                                            i += 1;
                                        }
                                    }
                                    text.push(v as u8);
                                }
                                other => text.push(other),
                            }
                        }
                        b'(' => {
                            depth += 1;
                            text.push(c);
                        }
                        b')' if depth == 0 => break,
                        b')' => {
                            depth -= 1;
                            text.push(c);
                        }
                        _ => text.push(c),
                    }
                }
                out.push(Token::Text(text));
            }
            b'<' if bytes.get(i + 1) == Some(&b'<') => {
                out.push(Token::Operator("<<".into()));
                i += 2;
            }
            b'>' if bytes.get(i + 1) == Some(&b'>') => {
                out.push(Token::Operator(">>".into()));
                i += 2;
            }
            b'<' => {
                let end = i + bytes[i..].iter().position(|&c| c == b'>').unwrap();
                let hex: Vec<u8> = bytes[i + 1..end]
                    .iter()
                    .copied()
                    .filter(|c| !c.is_ascii_whitespace())
                    .collect();
                let text = hex
                    .chunks(2)
                    .map(|pair| {
                        let mut pair = pair.to_vec();
                        pair.resize(2, b'0');
                        u8::from_str_radix(std::str::from_utf8(&pair).unwrap(), 16).unwrap()
                    })
                    .collect();
                out.push(Token::Text(text));
                i = end + 1;
            }
            _ => {
                let end = i + bytes[i..]
                    .iter()
                    .position(|c| c.is_ascii_whitespace() || b"/[]()<>".contains(c))
                    .unwrap_or(bytes.len() - i);
                let word = String::from_utf8_lossy(&bytes[i..end]).into_owned();
                out.push(match word.parse::<f64>() {
                    Ok(n) => Token::Number(n),
                    Err(_) => Token::Operator(word),
                });
                i = end.max(i + 1);
            }
        }
    }
    out
}
