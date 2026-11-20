//! Splits streamed reply text into speakable sentences (port of `SentenceChunker` in agent.ts):
//! a sentence ends at . ! ? … : ; followed by whitespace, or at a newline.

pub struct SentenceChunker {
    buf: String,
}

impl SentenceChunker {
    pub fn new() -> Self {
        Self { buf: String::new() }
    }

    /// Feed a delta; returns any complete sentences.
    pub fn push(&mut self, delta: &str) -> Vec<String> {
        self.buf.push_str(delta);
        let mut out = Vec::new();
        loop {
            let Some(end) = self.sentence_end() else { break };
            let sentence: String = self.buf[..end].trim().to_string();
            if sentence.chars().count() < 2 {
                break;
            }
            self.buf.drain(..end);
            out.push(sentence);
        }
        out
    }

    pub fn flush(&mut self) -> Option<String> {
        let rest = self.buf.trim().to_string();
        self.buf.clear();
        (!rest.is_empty()).then_some(rest)
    }

    /// Byte index just past the first sentence boundary, if any.
    fn sentence_end(&self) -> Option<usize> {
        let mut iter = self.buf.char_indices().peekable();
        while let Some((i, c)) = iter.next() {
            if c == '\n' {
                return Some(i + 1);
            }
            if matches!(c, '.' | '!' | '?' | '…' | ':' | ';') {
                if let Some(&(_, next)) = iter.peek() {
                    if next.is_whitespace() {
                        return Some(i + c.len_utf8());
                    }
                }
            }
        }
        None
    }
}

impl Default for SentenceChunker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_like_the_typescript_version() {
        let mut c = SentenceChunker::new();
        let mut got = c.push("Good evening, sir. It's 3.5 deg");
        got.extend(c.push("rees… and windy! Shall I"));
        got.extend(c.push(" continue?"));
        assert_eq!(got, vec!["Good evening, sir.", "It's 3.5 degrees…", "and windy!"]);
        assert_eq!(c.flush().as_deref(), Some("Shall I continue?"));
    }
}
