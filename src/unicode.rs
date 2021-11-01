/* [[[cog
# pip install cogapp
# cog -Ur unicode.rs
import cog

# http://www.unicode.org/reports/tr44/#General_Category_Values
NON_PRINTABLE_CATEGORIES = set("Cc Cf Cs Co Cn Zl Zp".split())

# https://www.unicode.org/Public/UNIDATA/UnicodeData.txt
codepoints = []
prev_code = None
for line in open("UnicodeData.txt"):
    code, name, category = line.split(";")[:3]
    if category in NON_PRINTABLE_CATEGORIES:
        code = int(code, 16)
        if name.endswith("First>"):
            prev_code = code
        elif name.endswith("Last>"):
            assert prev_code is not None
            for code in range(prev_code, code + 1):
                codepoints.append(code)
            prev_code = None
        else:
            codepoints.append(code)

codepoints = sorted(codepoints)
ranges = []
start = 0
for i, code in enumerate(codepoints):
    if i == 0:
        start = code
        continue
    prev = codepoints[i - 1]
    if code != prev + 1 or i + 1 == len(codepoints):
        # range start..=prev
        ranges.append((start, prev))
        start = code

cog.out("static NEED_ESCAPE_RANGES: [(u32, u32); %s] = [\n" % (len(ranges)))
for (start, end) in ranges:
    cog.out("    (0x%x, 0x%x),\n" % (start, end))
cog.outl("];\n")
]]] */
static NEED_ESCAPE_RANGES: [(u32, u32); 26] = [
    (0x0, 0x1f),
    (0x7f, 0x9f),
    (0xad, 0xad),
    (0x600, 0x605),
    (0x61c, 0x61c),
    (0x6dd, 0x6dd),
    (0x70f, 0x70f),
    (0x890, 0x891),
    (0x8e2, 0x8e2),
    (0x180e, 0x180e),
    (0x200b, 0x200f),
    (0x2028, 0x202e),
    (0x2060, 0x2064),
    (0x2066, 0x206f),
    (0xd800, 0xf8ff),
    (0xfeff, 0xfeff),
    (0xfff9, 0xfffb),
    (0x110bd, 0x110bd),
    (0x110cd, 0x110cd),
    (0x13430, 0x13438),
    (0x1bca0, 0x1bca3),
    (0x1d173, 0x1d17a),
    (0xe0001, 0xe0001),
    (0xe0020, 0xe007f),
    (0xf0000, 0xffffd),
    (0x100000, 0x10fffc),
];

/* [[[end]]] */

/// Test if a character needs escaping (control code, or multi-line separator).
pub(crate) fn need_escape(ch: char) -> bool {
    let v = ch as u32;
    let i = match NEED_ESCAPE_RANGES.binary_search_by_key(&v, |(_start, end)| *end) {
        Ok(_i) => return true,
        Err(i) => i,
    };
    if let Some((start, end)) = NEED_ESCAPE_RANGES.get(i) {
        debug_assert!(v <= *end);
        v >= *start
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_basic() {
        assert!(!need_escape('a'));
        assert!(!need_escape('字'));
        assert!(!need_escape('😀'));
        assert!(need_escape('\n'));
        assert!(need_escape('\u{08e2}'));
        assert!(need_escape('\u{f230}'));
    }
}
