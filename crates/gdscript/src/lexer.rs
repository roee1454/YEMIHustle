//! Turns GDScript source into tokens, with indentation as explicit `Indent` / `Dedent`.

use crate::Error;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    Int(i64),
    Float(f64),
    Str(String),
    /// `$Path/To/Node` or `$"quoted"`.
    NodePath(String),
    Op(&'static str),
    Newline,
    Indent,
    Dedent,
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub line: usize,
}

const OPS: [&str; 46] = [
    "**=", "<<=", ">>=", "->", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "**", "<<", ">>",
    ":=", "+", "-", "*", "/", "%", "=", "<", ">", "!", "&", "|", "^", "~", "(", ")", "[", "]", "{", "}", ",", ":", ".", ";", "@",
];

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

pub fn lex(src: &str) -> Result<Vec<Token>, Error> {
    let chars: Vec<char> = src.replace("\r\n", "\n").replace('\r', "\n").chars().collect();
    let mut out: Vec<Token> = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;
    let mut indents: Vec<usize> = vec![0];
    let mut depth = 0i32; // open brackets: newlines and indentation do not count inside
    let mut at_line_start = true;
    let push = |out: &mut Vec<Token>, tok: Tok, line: usize| out.push(Token { tok, line });

    while i < chars.len() {
        if at_line_start && depth == 0 {
            // measure the indentation of this line
            let mut col = 0usize;
            let mut j = i;
            while j < chars.len() && (chars[j] == ' ' || chars[j] == '\t') {
                col += if chars[j] == '\t' { 1000 } else { 1 }; // a tab outranks any run of spaces
                j += 1;
            }
            // blank and comment-only lines do not change the indentation
            if j >= chars.len() || chars[j] == '\n' || chars[j] == '#' {
                while j < chars.len() && chars[j] != '\n' {
                    j += 1;
                }
                i = j + 1;
                line += 1;
                continue;
            }
            // normalise: a run of tabs is by far the common case; count spaces and tabs alike per level
            let tabs = chars[i..j].iter().filter(|c| **c == '\t').count();
            let spaces = j - i - tabs;
            let level = tabs * 1000 + spaces;
            let _ = col;
            if level > *indents.last().expect("one level") {
                indents.push(level);
                push(&mut out, Tok::Indent, line);
            } else {
                while level < *indents.last().expect("one level") {
                    indents.pop();
                    push(&mut out, Tok::Dedent, line);
                }
                if level != *indents.last().expect("one level") {
                    return Err(Error::Parse { line, msg: "inconsistent indentation".into() });
                }
            }
            i = j;
            at_line_start = false;
            continue;
        }
        let c = chars[i];
        match c {
            ' ' | '\t' => i += 1,
            '\n' => {
                line += 1;
                i += 1;
                if depth == 0 {
                    if !matches!(out.last().map(|t| &t.tok), Some(Tok::Newline) | None) {
                        push(&mut out, Tok::Newline, line - 1);
                    }
                    at_line_start = true;
                }
            }
            '\\' if chars.get(i + 1) == Some(&'\n') => {
                i += 2;
                line += 1;
            }
            '#' => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '"' | '\'' => {
                let q = c;
                let triple = chars.get(i + 1) == Some(&q) && chars.get(i + 2) == Some(&q);
                i += if triple { 3 } else { 1 };
                let mut s = String::new();
                loop {
                    let Some(&ch) = chars.get(i) else { return Err(Error::Parse { line, msg: "unterminated string".into() }) };
                    if triple {
                        if ch == q && chars.get(i + 1) == Some(&q) && chars.get(i + 2) == Some(&q) {
                            i += 3;
                            break;
                        }
                    } else if ch == q {
                        i += 1;
                        break;
                    } else if ch == '\n' {
                        return Err(Error::Parse { line, msg: "newline in string".into() });
                    }
                    if ch == '\\' {
                        i += 1;
                        let e = chars.get(i).copied().unwrap_or('\\');
                        s.push(match e {
                            'n' => '\n',
                            't' => '\t',
                            'r' => '\r',
                            '\n' => {
                                line += 1;
                                i += 1;
                                continue;
                            }
                            other => other,
                        });
                        i += 1;
                        continue;
                    }
                    if ch == '\n' {
                        line += 1;
                    }
                    s.push(ch);
                    i += 1;
                }
                push(&mut out, Tok::Str(s), line);
            }
            '$' => {
                i += 1;
                let mut s = String::new();
                if matches!(chars.get(i), Some('"') | Some('\'')) {
                    let q = chars[i];
                    i += 1;
                    while i < chars.len() && chars[i] != q {
                        s.push(chars[i]);
                        i += 1;
                    }
                    i += 1;
                } else {
                    loop {
                        while i < chars.len() && (is_ident(chars[i]) || chars[i] == '/' || chars[i] == '%') {
                            s.push(chars[i]);
                            i += 1;
                        }
                        // decompilers write `$StateMachine / Active / Hitbox`: the spaced `/` and
                        // the name after it are still the path
                        let mut j = i;
                        while matches!(chars.get(j), Some(' ')) {
                            j += 1;
                        }
                        if chars.get(j) != Some(&'/') {
                            break;
                        }
                        let mut k = j + 1;
                        while matches!(chars.get(k), Some(' ')) {
                            k += 1;
                        }
                        if !chars.get(k).is_some_and(|c| c.is_alphabetic() || *c == '_') {
                            break;
                        }
                        s.push('/');
                        i = k;
                    }
                }
                // `$"%Name" / Child` style continuations are rare; a following `/Child` is read above
                push(&mut out, Tok::NodePath(s), line);
            }
            '0'..='9' => {
                let start = i;
                if c == '0' && matches!(chars.get(i + 1), Some('x') | Some('X')) {
                    i += 2;
                    while i < chars.len() && (chars[i].is_ascii_hexdigit() || chars[i] == '_') {
                        i += 1;
                    }
                    let text: String = chars[start + 2..i].iter().filter(|c| **c != '_').collect();
                    push(&mut out, Tok::Int(i64::from_str_radix(&text, 16).map_err(|_| Error::Parse { line, msg: "bad hex number".into() })?), line);
                    continue;
                }
                let mut float = false;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
                    i += 1;
                }
                if chars.get(i) == Some(&'.') && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()) {
                    float = true;
                    i += 1;
                    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
                        i += 1;
                    }
                } else if chars.get(i) == Some(&'.') && !chars.get(i + 1).is_some_and(|d| is_ident_start(*d)) {
                    float = true; // `1.`
                    i += 1;
                }
                if matches!(chars.get(i), Some('e') | Some('E')) && (chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()) || (matches!(chars.get(i + 1), Some('-') | Some('+')) && chars.get(i + 2).is_some_and(|d| d.is_ascii_digit()))) {
                    float = true;
                    i += 2;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let text: String = chars[start..i].iter().filter(|c| **c != '_').collect();
                if float {
                    push(&mut out, Tok::Float(text.parse().map_err(|_| Error::Parse { line, msg: format!("bad number {text}") })?), line);
                } else {
                    push(&mut out, Tok::Int(text.parse().map_err(|_| Error::Parse { line, msg: format!("bad number {text}") })?), line);
                }
            }
            '.' if chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()) => {
                let start = i;
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                push(&mut out, Tok::Float(format!("0{text}").parse().map_err(|_| Error::Parse { line, msg: "bad number".into() })?), line);
            }
            c if is_ident_start(c) => {
                let start = i;
                while i < chars.len() && is_ident(chars[i]) {
                    i += 1;
                }
                push(&mut out, Tok::Ident(chars[start..i].iter().collect()), line);
            }
            _ => {
                let rest: String = chars[i..chars.len().min(i + 3)].iter().collect();
                let Some(op) = OPS.iter().find(|o| rest.starts_with(**o)) else {
                    return Err(Error::Parse { line, msg: format!("unexpected character {c:?}") });
                };
                match *op {
                    "(" | "[" | "{" => depth += 1,
                    ")" | "]" | "}" => depth -= 1,
                    _ => {}
                }
                i += op.chars().count();
                push(&mut out, Tok::Op(op), line);
            }
        }
    }
    if !matches!(out.last().map(|t| &t.tok), Some(Tok::Newline) | None) {
        push(&mut out, Tok::Newline, line);
    }
    while indents.len() > 1 {
        indents.pop();
        push(&mut out, Tok::Dedent, line);
    }
    push(&mut out, Tok::Eof, line);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Tok> {
        lex(s).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn indentation_becomes_tokens() {
        let t = toks("func f():\n\tif x:\n\t\treturn 1\n\treturn 2\n");
        assert_eq!(t.iter().filter(|t| **t == Tok::Indent).count(), 2);
        assert_eq!(t.iter().filter(|t| **t == Tok::Dedent).count(), 2);
    }

    #[test]
    fn brackets_join_lines() {
        let t = toks("var a = [1,\n  2,\n 3]\n");
        assert_eq!(t.iter().filter(|t| **t == Tok::Newline).count(), 1);
    }

    #[test]
    fn numbers_and_strings() {
        assert_eq!(toks("1.5"), vec![Tok::Float(1.5), Tok::Newline, Tok::Eof]);
        assert_eq!(toks("0xFF"), vec![Tok::Int(255), Tok::Newline, Tok::Eof]);
        assert_eq!(toks("\"a\\nb\""), vec![Tok::Str("a\nb".into()), Tok::Newline, Tok::Eof]);
        assert_eq!(toks("$\"%Box\""), vec![Tok::NodePath("%Box".into()), Tok::Newline, Tok::Eof]);
        assert_eq!(toks("$StateMachine / Active / Hitbox"), vec![Tok::NodePath("StateMachine/Active/Hitbox".into()), Tok::Newline, Tok::Eof]);
        // a division after a path stays a division
        assert_eq!(toks("$A.x / 2")[..2], [Tok::NodePath("A".into()), Tok::Op(".")]);
    }
}
