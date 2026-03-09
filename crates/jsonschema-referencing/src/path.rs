/// Escape a key into a JSON Pointer segment: `~` → `~0`, `/` → `~1`.
///
/// Appends the escaped form of `value` directly to `buffer`.
pub fn write_escaped_str(buffer: &mut String, value: &str) {
    match value.find(['~', '/']) {
        Some(mut escape_idx) => {
            let mut remaining = value;

            // Loop through the string to replace `~` and `/`
            loop {
                let (before, after) = remaining.split_at(escape_idx);
                // Copy everything before the escape char
                buffer.push_str(before);

                // Append the appropriate escape sequence
                match after.as_bytes()[0] {
                    b'~' => buffer.push_str("~0"),
                    b'/' => buffer.push_str("~1"),
                    _ => unreachable!(),
                }

                // Move past the escaped character
                remaining = &after[1..];

                // Find the next `~` or `/` to continue escaping
                if let Some(next_escape_idx) = remaining.find(['~', '/']) {
                    escape_idx = next_escape_idx;
                } else {
                    // Append any remaining part of the string
                    buffer.push_str(remaining);
                    break;
                }
            }
        }
        None => {
            // If no escape characters are found, append the segment as is
            buffer.push_str(value);
        }
    }
}

#[inline]
pub fn write_index(buffer: &mut String, idx: usize) {
    let mut itoa_buffer = itoa::Buffer::new();
    buffer.push_str(itoa_buffer.format(idx));
}
