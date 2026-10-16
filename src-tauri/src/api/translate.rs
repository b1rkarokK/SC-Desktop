//! Translation of lyrics line by line («Перевод» in the lyrics panel). The free translate endpoint of the web
//! translator, no key and no server of ours; the lines go in one request when they fit (the answer keeps the line
//! breaks), otherwise one by one. The result is kept per track (kv "lyr:tr:<id>:<lang>").

use futures::StreamExt;
use serde_json::Value;

use crate::{error::AppResult, net::client::HttpClient};

const ENDPOINT: &str = "https://translate.googleapis.com/translate_a/single";
/// characters of the text in one request (the address stays short)
const BATCH_CHARS: usize = 1200;

fn text_of(v: &Value) -> String {
    v.get(0)
        .and_then(Value::as_array)
        .map(|segs| segs.iter().filter_map(|s| s.get(0).and_then(Value::as_str)).collect::<String>())
        .unwrap_or_default()
}

async fn ask(http: &HttpClient, text: &str, to: &str) -> AppResult<String> {
    let url = format!("{ENDPOINT}?client=gtx&sl=auto&tl={}&dt=t&q={}", urlencoding::encode(to), urlencoding::encode(text));
    let v: Value = http.get_json_with(&url, &[]).await?;
    Ok(text_of(&v))
}

/// One translated line for each of `lines` (an empty line stays empty).
pub async fn translate_lines(http: &HttpClient, lines: &[String], to: &str) -> AppResult<Vec<String>> {
    let mut out: Vec<String> = vec![String::new(); lines.len()];
    // groups of neighbouring non-empty lines that fit in one request
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    let mut size = 0;
    for (i, l) in lines.iter().enumerate() {
        if l.trim().is_empty() {
            continue;
        }
        if size + l.len() > BATCH_CHARS && !cur.is_empty() {
            groups.push(std::mem::take(&mut cur));
            size = 0;
        }
        size += l.len() + 1;
        cur.push(i);
    }
    if !cur.is_empty() {
        groups.push(cur);
    }
    let done: Vec<(Vec<usize>, Vec<String>)> = futures::stream::iter(groups)
        .map(|g| async move {
            let joined = g.iter().map(|&i| lines[i].replace('\n', " ")).collect::<Vec<_>>().join("\n");
            let whole = ask(http, &joined, to).await.unwrap_or_default();
            let parts: Vec<String> = whole.split('\n').map(|s| s.trim().to_owned()).collect();
            if parts.len() == g.len() && parts.iter().all(|p| !p.is_empty()) {
                return (g, parts);
            }
            // the breaks did not survive: line by line
            let mut single = Vec::with_capacity(g.len());
            for &i in &g {
                single.push(ask(http, &lines[i], to).await.unwrap_or_default().trim().to_owned());
            }
            (g, single)
        })
        .buffered(3)
        .collect()
        .await;
    for (g, parts) in done {
        for (i, p) in g.into_iter().zip(parts) {
            out[i] = p;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_answer_is_read_from_its_segments() {
        let v: Value = serde_json::from_str(r#"[[["Привет\n","Hello\n",null,null,1],["Мир","World",null,null,1]],null,"en"]"#).unwrap();
        assert_eq!(text_of(&v), "Привет\nМир");
    }
}
