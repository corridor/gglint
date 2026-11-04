//! A small JSON writer for the formats gglint prints: the lineage graph and the
//! diagrams.
//!
//! Written by hand rather than through serde: the formats are small and fixed, and the
//! crate has no other use for serde. Every value is written followed by a comma, and
//! [`Json::end_object`] and [`Json::end_array`] replace the last one.

use std::fmt::Write;

pub(crate) struct Json(pub(crate) String);

impl Json {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self(String::with_capacity(capacity))
    }

    pub(crate) fn key(&mut self, key: &str) {
        // Keys are written here, never taken from the code, so they need no escaping.
        self.0.push('"');
        self.0.push_str(key);
        self.0.push_str("\":");
    }

    pub(crate) fn string(&mut self, key: &str, value: &str) {
        self.key(key);
        self.escaped(value);
        self.0.push(',');
    }

    /// `value` in quotes, escaped.
    fn escaped(&mut self, value: &str) {
        self.0.push('"');
        let bytes = value.as_bytes();
        // Almost all text needs no escaping, and this check compiles to a fast loop.
        if !bytes.iter().any(|&byte| byte < 0x20 || byte == b'"' || byte == b'\\') {
            self.0.push_str(value);
            self.0.push('"');
            return;
        }
        let mut start = 0;
        for (index, &byte) in bytes.iter().enumerate() {
            let escape = match byte {
                b'"' => "\\\"",
                b'\\' => "\\\\",
                b'\n' => "\\n",
                b'\r' => "\\r",
                b'\t' => "\\t",
                0..=0x1f => "",
                _ => continue,
            };
            self.0.push_str(&value[start..index]);
            if escape.is_empty() {
                let _ = write!(self.0, "\\u{byte:04x}");
            } else {
                self.0.push_str(escape);
            }
            start = index + 1;
        }
        self.0.push_str(&value[start..]);
        self.0.push('"');
    }

    pub(crate) fn optional(&mut self, key: &str, value: Option<&str>) {
        match value {
            Some(value) => self.string(key, value),
            None => self.null(key),
        }
    }

    pub(crate) fn number(&mut self, key: &str, value: usize) {
        self.key(key);
        let _ = write!(self.0, "{value},");
    }

    pub(crate) fn null(&mut self, key: &str) {
        self.key(key);
        self.0.push_str("null,");
    }

    /// Replaces the trailing comma every value is written with, and closes the object.
    pub(crate) fn end_object(&mut self) {
        if self.0.ends_with(',') {
            self.0.pop();
        }
        self.0.push('}');
    }

    pub(crate) fn boolean(&mut self, key: &str, value: bool) {
        self.key(key);
        self.0.push_str(if value { "true," } else { "false," });
    }

    /// An array of strings.
    pub(crate) fn strings<'s>(&mut self, key: &str, values: impl IntoIterator<Item = &'s str>) {
        self.begin_array(key);
        for value in values {
            self.element(value);
        }
        self.end_array();
    }

    /// A string in an array.
    pub(crate) fn element(&mut self, value: &str) {
        self.escaped(value);
        self.0.push(',');
    }

    /// Closes an object that is a value in another object or array, followed by a
    /// comma like every other value.
    pub(crate) fn close(&mut self) {
        self.end_object();
        self.0.push(',');
    }

    pub(crate) fn begin_object(&mut self) {
        self.0.push('{');
    }

    /// `"key":[`
    pub(crate) fn begin_array(&mut self, key: &str) {
        self.key(key);
        self.0.push('[');
    }

    /// Replaces the trailing comma every value is written with, and closes the array.
    pub(crate) fn end_array(&mut self) {
        if self.0.ends_with(',') {
            self.0.pop();
        }
        self.0.push_str("],");
    }
}
