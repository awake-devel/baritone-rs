// Written for this project from the public NBT specification (Minecraft's `NbtIo.readCompressed` and `CompoundTag` are the behaviour it follows).
// Part of the schematic formats ported from cabaletta/baritone at 1b8e87c: src/main/java/baritone/utils/schematic/format/. LGPL-3.0-or-later, see lib.rs.
//
// An NBT reader and the getters of `CompoundTag` that the formats use. No NBT code of the game or of anyone else was copied.
//
// - Big-endian. Tags 0 to 12. Strings are modified UTF-8 (`DataInput.readUTF`): NUL is C0 80, a supplementary character is a surrogate pair of
//   two 3-byte units. A string decodes to a Rust `String`; an unpaired surrogate becomes U+FFFD (Java keeps the lone UTF-16 unit).
// - The root must be a compound with a name (the name is read and thrown away). Data after the root is ignored.
// - Depth: a compound or a list nests one level; more than 512 is an error, as `NbtAccounter` has it.
// - Upstream reads with `NbtAccounter.unlimitedHeap()`: no size limit. This port keeps a budget of its own (see D47): the gzip layer is held to
//   `max_inflated_bytes`, and the tags read must not cost more than that many bytes of memory (an estimate built from `size_of` of the tag, the
//   entry and the key's two copies, with the slack of vectors that grow by doubling: an upper bound, D50). Every length is checked against
//   the bytes that are left before anything is allocated.
// - `Compound` keeps the order of the file and can tell the iteration order of the `java.util.HashMap` that Minecraft's `CompoundTag` fills.
use super::ParseError;
use crate::cache::gzip;
use crate::mth;
use std::collections::HashMap;
use std::sync::OnceLock;

/// What the memory budget charges (D47, D50). A tag has no cost of its own beyond the slot it sits in: a slot of a list (`2 * TAG_SIZE`,
/// because a vector grown by doubling holds up to twice as many slots as tags) or an entry of a compound. The heap parts are charged where
/// they are made. The numbers come from `size_of`, so they follow the types.
const TAG_SIZE: usize = std::mem::size_of::<Tag>();
/// One compound entry: the `Entry` in the vector and the key and index slot in the map, both of which may be twice as big as the count
/// needs, plus the second copy of the key (charged with `2 * key.len()` where the entry is read).
const ENTRY_OVERHEAD: usize = 2 * std::mem::size_of::<Entry>() + 2 * (std::mem::size_of::<String>() + std::mem::size_of::<usize>() + 8);
/// The allocation header of a heap part (an array, a string, the vector of a list or a compound).
const ARRAY_OVERHEAD: usize = 16;

/// The deepest nesting `NbtAccounter` allows.
pub const MAX_DEPTH: u32 = 512;

/// One NBT tag. `List` holds the element type id and the elements.
#[derive(Clone, Debug, PartialEq)]
pub enum Tag {
    End,
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List(u8, Vec<Tag>),
    Compound(Compound),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Tag {
    /// `Tag.getId`.
    pub fn id(&self) -> u8 {
        match self {
            Tag::End => 0,
            Tag::Byte(_) => 1,
            Tag::Short(_) => 2,
            Tag::Int(_) => 3,
            Tag::Long(_) => 4,
            Tag::Float(_) => 5,
            Tag::Double(_) => 6,
            Tag::ByteArray(_) => 7,
            Tag::String(_) => 8,
            Tag::List(..) => 9,
            Tag::Compound(_) => 10,
            Tag::IntArray(_) => 11,
            Tag::LongArray(_) => 12,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Entry {
    key: String,
    /// `String.hashCode` of the key, taken from the UTF-16 units as the file had them.
    hash: i32,
    tag: Tag,
}

/// `CompoundTag`: named tags. Entries stay in the order they were first put.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Compound {
    entries: Vec<Entry>,
    index: HashMap<String, usize>,
}

/// Like `String.hashCode`: over the UTF-16 units, `31 * h + c`, wrapping.
pub fn java_string_hash(s: &str) -> i32 {
    s.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
}

fn empty_compound() -> &'static Compound {
    static EMPTY: OnceLock<Compound> = OnceLock::new();
    EMPTY.get_or_init(Compound::default)
}

impl Compound {
    pub fn new() -> Compound {
        Compound::default()
    }

    /// `CompoundTag.put`. A key that is there already keeps its place and gets the new tag, as a `HashMap` does.
    pub fn insert(&mut self, key: &str, tag: Tag) {
        let hash = java_string_hash(key);
        self.insert_hashed(key.to_string(), hash, tag);
    }

    fn insert_hashed(&mut self, key: String, hash: i32, tag: Tag) {
        if let Some(&i) = self.index.get(&key) {
            self.entries[i].tag = tag;
            return;
        }
        self.index.insert(key.clone(), self.entries.len());
        self.entries.push(Entry { key, hash, tag });
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `CompoundTag.get`.
    pub fn get(&self, key: &str) -> Option<&Tag> {
        self.index.get(key).map(|&i| &self.entries[i].tag)
    }

    /// Like `CompoundTag.contains(String)`: a tag of any type is there.
    pub fn contains(&self, key: &str) -> bool {
        self.index.contains_key(key)
    }

    /// The keys in the order of the file.
    pub fn keys(&self) -> Vec<&str> {
        self.entries.iter().map(|e| e.key.as_str()).collect()
    }

    /// `CompoundTag.getAllKeys` as it iterates in 1.21.4, where the tags are a `java.util.HashMap<String, Tag>` filled in file order.
    /// The table has 16 buckets and doubles when the size goes above three quarters of it; a key goes to bucket `h ^ (h >>> 16)` masked, and
    /// inside a bucket the keys stay in the order they came (a resize splits a bucket without reordering it). So the order is the bucket
    /// index, then the place in the file. Not modelled: a bucket with 8 or more keys is turned into a tree (or the table is doubled early),
    /// which needs 8 keys with the same bucket; real files do not have that (D47).
    pub fn keys_java_order(&self) -> Vec<&str> {
        let n = self.entries.len();
        let mut cap = 16usize;
        while n > cap / 4 * 3 {
            cap *= 2;
        }
        let mut order: Vec<(usize, usize)> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let h = e.hash as u32;
                (((h ^ (h >> 16)) as usize) & (cap - 1), i)
            })
            .collect();
        order.sort_unstable();
        order.into_iter().map(|(_, i)| self.entries[i].key.as_str()).collect()
    }

    /// `CompoundTag.getInt` in 1.21.4: a number of any type, cast as `NumericTag.getAsInt` casts it (a long wraps, a float or double goes
    /// through `Mth.floor`, which saturates and turns NaN into 0); anything else, or no tag, is 0. (26.3 returns an `Optional`, and the
    /// callers there give the same 0 for a missing key through `getIntOr`.)
    pub fn get_int(&self, key: &str) -> i32 {
        match self.get(key) {
            Some(Tag::Byte(v)) => i32::from(*v),
            Some(Tag::Short(v)) => i32::from(*v),
            Some(Tag::Int(v)) => *v,
            Some(Tag::Long(v)) => *v as i32,
            Some(Tag::Float(v)) => mth::floor(f64::from(*v)),
            Some(Tag::Double(v)) => mth::floor(*v),
            _ => 0,
        }
    }

    /// Like `CompoundTag.getString`: "" unless the tag is a string.
    pub fn get_string(&self, key: &str) -> &str {
        match self.get(key) {
            Some(Tag::String(s)) => s,
            _ => "",
        }
    }

    /// Like `CompoundTag.getByteArray`: empty unless the tag is a byte array.
    pub fn get_byte_array(&self, key: &str) -> &[i8] {
        match self.get(key) {
            Some(Tag::ByteArray(v)) => v,
            _ => &[],
        }
    }

    /// Like `CompoundTag.getLongArray`: empty unless the tag is a long array.
    pub fn get_long_array(&self, key: &str) -> &[i64] {
        match self.get(key) {
            Some(Tag::LongArray(v)) => v,
            _ => &[],
        }
    }

    /// Like `CompoundTag.getCompound`: an empty compound unless the tag is a compound.
    pub fn get_compound(&self, key: &str) -> &Compound {
        match self.get(key) {
            Some(Tag::Compound(c)) => c,
            _ => empty_compound(),
        }
    }

    /// Like `CompoundTag.getList(key, 10)`: the compounds of the list; empty unless the tag is a list of compounds (an empty list of any type is empty).
    pub fn get_compound_list(&self, key: &str) -> Vec<&Compound> {
        match self.get(key) {
            Some(Tag::List(10, items)) => items
                .iter()
                .filter_map(|t| match t {
                    Tag::Compound(c) => Some(c),
                    _ => None,
                })
                .collect(),
            _ => vec![],
        }
    }
}

/// Like `NbtIo.readCompressed`: a gzip file holding a named root compound. The inflated data is held to `max_inflated_bytes`, and so is the memory the tags are estimated to use.
pub fn read_compressed(bytes: &[u8], max_inflated_bytes: usize) -> Result<Compound, ParseError> {
    let data = gzip::decompress(bytes, max_inflated_bytes).map_err(ParseError)?;
    read_uncompressed(&data, max_inflated_bytes)
}

/// The part of `NbtIo.readCompressed` after the gzip layer (`NbtIo.read(DataInput, NbtAccounter)`): the root tag must be a named compound.
pub fn read_uncompressed(data: &[u8], max_bytes: usize) -> Result<Compound, ParseError> {
    let mut r = Reader { data, pos: 0, depth: 0, budget: max_bytes };
    let id = r.byte()?;
    if id != 10 {
        // also an end tag as the root: upstream's `readUnnamedTag` returns it and `read` refuses it for the same reason
        return Err(ParseError("Root tag must be a named compound tag".to_string()));
    }
    r.string()?;
    match r.load(10)? {
        Tag::Compound(c) => Ok(c),
        _ => Err(ParseError("Root tag must be a named compound tag".to_string())),
    }
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    depth: u32,
    budget: usize,
}

fn err<T>(msg: &str) -> Result<T, ParseError> {
    Err(ParseError(msg.to_string()))
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ParseError> {
        if n > self.data.len() - self.pos {
            return err("Unexpected end of NBT data");
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn byte(&mut self) -> Result<u8, ParseError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, ParseError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn i32(&mut self) -> Result<i32, ParseError> {
        let b = self.take(4)?;
        Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i64(&mut self) -> Result<i64, ParseError> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(i64::from_be_bytes(a))
    }

    /// Takes `n` bytes of the budget.
    fn charge(&mut self, n: usize) -> Result<(), ParseError> {
        match self.budget.checked_sub(n) {
            Some(left) => {
                self.budget = left;
                Ok(())
            }
            None => err("NBT data is too large for this port's limit"),
        }
    }

    fn push_depth(&mut self) -> Result<(), ParseError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return err("Tried to read NBT tag with too high complexity, depth > 512");
        }
        Ok(())
    }

    /// `DataInputStream.readUTF`, and the hash of the string as Java would take it.
    fn string_hashed(&mut self) -> Result<(String, i32), ParseError> {
        let len = usize::from(self.u16()?);
        let b = self.take(len)?;
        let mut units: Vec<u16> = Vec::with_capacity(len);
        let mut i = 0;
        while i < len {
            let c = b[i];
            match c >> 4 {
                0..=7 => {
                    units.push(u16::from(c));
                    i += 1;
                }
                12 | 13 => {
                    if i + 2 > len {
                        return err("malformed input: partial character at end");
                    }
                    let c2 = b[i + 1];
                    if c2 & 0xC0 != 0x80 {
                        return err("malformed input around byte");
                    }
                    units.push((u16::from(c & 0x1F) << 6) | u16::from(c2 & 0x3F));
                    i += 2;
                }
                14 => {
                    if i + 3 > len {
                        return err("malformed input: partial character at end");
                    }
                    let (c2, c3) = (b[i + 1], b[i + 2]);
                    if c2 & 0xC0 != 0x80 || c3 & 0xC0 != 0x80 {
                        return err("malformed input around byte");
                    }
                    units.push((u16::from(c & 0x0F) << 12) | (u16::from(c2 & 0x3F) << 6) | u16::from(c3 & 0x3F));
                    i += 3;
                }
                _ => return err("malformed input around byte"),
            }
        }
        let hash = units.iter().fold(0i32, |h, &u| h.wrapping_mul(31).wrapping_add(i32::from(u)));
        Ok((String::from_utf16_lossy(&units), hash))
    }

    fn string(&mut self) -> Result<String, ParseError> {
        Ok(self.string_hashed()?.0)
    }

    /// The length of an array tag, checked against what is left. The elements are `width` bytes each.
    fn array_len(&mut self, width: usize) -> Result<usize, ParseError> {
        let n = self.i32()?;
        if n < 0 {
            return err("negative array length in NBT data");
        }
        let n = n as usize;
        if n > (self.data.len() - self.pos) / width {
            return err("Unexpected end of NBT data");
        }
        self.charge(ARRAY_OVERHEAD + n * width)?;
        Ok(n)
    }

    /// `TagTypes.getType(id).load`. The two nesting kinds are functions of their own, so that the frames of a deep file stay small.
    fn load(&mut self, id: u8) -> Result<Tag, ParseError> {
        match id {
            9 => self.load_list(),
            10 => self.load_compound(),
            _ => self.load_flat(id),
        }
    }

    fn load_list(&mut self) -> Result<Tag, ParseError> {
        self.push_depth()?;
        self.charge(ARRAY_OVERHEAD)?;
        let elem = self.byte()?;
        let len = self.i32()?;
        if elem == 0 && len > 0 {
            return err("Missing type on ListTag");
        }
        // a negative length is an empty list, as in the game's loop
        let len = len.max(0) as usize;
        let mut items = Vec::with_capacity(len.min(128));
        for _ in 0..len {
            // the slot in the vector, which has grown by doubling and may hold up to twice as many slots as tags
            self.charge(2 * TAG_SIZE)?;
            items.push(self.load(elem)?);
        }
        self.depth -= 1;
        Ok(Tag::List(elem, items))
    }

    fn load_compound(&mut self) -> Result<Tag, ParseError> {
        self.push_depth()?;
        self.charge(ARRAY_OVERHEAD)?;
        let mut c = Compound::new();
        loop {
            let t = self.byte()?;
            if t == 0 {
                break;
            }
            let (key, hash) = self.string_hashed()?;
            self.charge(ENTRY_OVERHEAD + 2 * key.len())?;
            let tag = self.load(t)?;
            c.insert_hashed(key, hash, tag);
        }
        self.depth -= 1;
        Ok(Tag::Compound(c))
    }

    /// The tags that do not nest.
    fn load_flat(&mut self, id: u8) -> Result<Tag, ParseError> {
        match id {
            1 => Ok(Tag::Byte(self.byte()? as i8)),
            2 => Ok(Tag::Short(self.u16()? as i16)),
            3 => Ok(Tag::Int(self.i32()?)),
            4 => Ok(Tag::Long(self.i64()?)),
            5 => Ok(Tag::Float(f32::from_bits(self.i32()? as u32))),
            6 => Ok(Tag::Double(f64::from_bits(self.i64()? as u64))),
            7 => {
                let n = self.array_len(1)?;
                Ok(Tag::ByteArray(self.take(n)?.iter().map(|&b| b as i8).collect()))
            }
            8 => {
                let s = self.string()?;
                self.charge(ARRAY_OVERHEAD + s.len())?;
                Ok(Tag::String(s))
            }
            11 => {
                let n = self.array_len(4)?;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(self.i32()?);
                }
                Ok(Tag::IntArray(v))
            }
            12 => {
                let n = self.array_len(8)?;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(self.i64()?);
                }
                Ok(Tag::LongArray(v))
            }
            _ => Err(ParseError(format!("Invalid tag id: {id}"))),
        }
    }
}

/// A small NBT writer for the tests of the formats: enough to build files in test code.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// Modified UTF-8 of a string.
    pub fn mutf8(s: &str) -> Vec<u8> {
        let mut out = vec![];
        for u in s.encode_utf16() {
            match u {
                1..=0x7F => out.push(u as u8),
                0 | 0x80..=0x7FF => {
                    out.push(0xC0 | (u >> 6) as u8);
                    out.push(0x80 | (u & 0x3F) as u8);
                }
                _ => {
                    out.push(0xE0 | (u >> 12) as u8);
                    out.push(0x80 | ((u >> 6) & 0x3F) as u8);
                    out.push(0x80 | (u & 0x3F) as u8);
                }
            }
        }
        out
    }

    fn put_string(out: &mut Vec<u8>, s: &str) {
        let b = mutf8(s);
        out.extend_from_slice(&(b.len() as u16).to_be_bytes());
        out.extend_from_slice(&b);
    }

    /// The payload of a tag (no id, no name).
    pub fn payload(tag: &Tag, out: &mut Vec<u8>) {
        match tag {
            Tag::End => {}
            Tag::Byte(v) => out.push(*v as u8),
            Tag::Short(v) => out.extend_from_slice(&v.to_be_bytes()),
            Tag::Int(v) => out.extend_from_slice(&v.to_be_bytes()),
            Tag::Long(v) => out.extend_from_slice(&v.to_be_bytes()),
            Tag::Float(v) => out.extend_from_slice(&v.to_bits().to_be_bytes()),
            Tag::Double(v) => out.extend_from_slice(&v.to_bits().to_be_bytes()),
            Tag::ByteArray(v) => {
                out.extend_from_slice(&(v.len() as i32).to_be_bytes());
                out.extend(v.iter().map(|&b| b as u8));
            }
            Tag::String(s) => put_string(out, s),
            Tag::List(t, items) => {
                out.push(*t);
                out.extend_from_slice(&(items.len() as i32).to_be_bytes());
                for i in items {
                    payload(i, out);
                }
            }
            Tag::Compound(c) => {
                for e in &c.entries {
                    out.push(e.tag.id());
                    put_string(out, &e.key);
                    payload(&e.tag, out);
                }
                out.push(0);
            }
            Tag::IntArray(v) => {
                out.extend_from_slice(&(v.len() as i32).to_be_bytes());
                for x in v {
                    out.extend_from_slice(&x.to_be_bytes());
                }
            }
            Tag::LongArray(v) => {
                out.extend_from_slice(&(v.len() as i32).to_be_bytes());
                for x in v {
                    out.extend_from_slice(&x.to_be_bytes());
                }
            }
        }
    }

    /// An uncompressed file: the root compound with an empty name.
    pub fn root(c: &Compound) -> Vec<u8> {
        let mut out = vec![10];
        put_string(&mut out, "");
        payload(&Tag::Compound(c.clone()), &mut out);
        out
    }

    /// A gzip file of the root compound.
    pub fn gz(c: &Compound) -> Vec<u8> {
        gzip::compress(&root(c), 1)
    }

    /// A compound from pairs.
    pub fn comp(pairs: Vec<(&str, Tag)>) -> Compound {
        let mut c = Compound::new();
        for (k, t) in pairs {
            c.insert(k, t);
        }
        c
    }

    pub fn s(v: &str) -> Tag {
        Tag::String(v.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    const BIG: usize = 1 << 28;

    fn read(c: &Compound) -> Compound {
        read_uncompressed(&root(c), BIG).unwrap()
    }

    #[test]
    fn every_tag_type_round_trips() {
        let c = comp(vec![
            ("b", Tag::Byte(-5)),
            ("sh", Tag::Short(-300)),
            ("i", Tag::Int(-70000)),
            ("l", Tag::Long(-5_000_000_000)),
            ("f", Tag::Float(1.5)),
            ("d", Tag::Double(-2.25)),
            ("ba", Tag::ByteArray(vec![1, -2, 3])),
            ("st", s("hello")),
            ("li", Tag::List(3, vec![Tag::Int(1), Tag::Int(2)])),
            ("empty", Tag::List(0, vec![])),
            ("co", Tag::Compound(comp(vec![("x", Tag::Byte(1))]))),
            ("ia", Tag::IntArray(vec![-1, 2])),
            ("la", Tag::LongArray(vec![i64::MIN, 7])),
        ]);
        assert_eq!(read(&c), c);
        assert_eq!(read_compressed(&gz(&c), BIG).unwrap(), c);
    }

    #[test]
    fn modified_utf8_nul_and_supplementary() {
        // NUL is two bytes, never one
        assert_eq!(mutf8("a\0b"), vec![b'a', 0xC0, 0x80, b'b']);
        // an emoji (U+1F600) is a surrogate pair, 3 bytes each
        let e = "x\u{1F600}y";
        let b = mutf8(e);
        assert_eq!(b.len(), 1 + 6 + 1);
        let c = comp(vec![(e, s(e)), ("nul\0key", s("a\0b"))]);
        let back = read(&c);
        assert_eq!(back.get_string(e), e);
        assert_eq!(back.get_string("nul\0key"), "a\0b");
        // the key hash is over UTF-16 units: the emoji counts twice
        assert_eq!(java_string_hash("\u{1F600}"), 0xD83D * 31 + 0xDE00);
    }

    #[test]
    fn modified_utf8_errors_and_lone_surrogates() {
        // a raw 4-byte UTF-8 sequence is malformed in modified UTF-8
        let mut data = vec![10, 0, 0, 8, 0, 1, b'k', 0, 4, 0xF0, 0x9F, 0x98, 0x80, 0];
        assert!(read_uncompressed(&data, BIG).is_err());
        // a lone high surrogate (ED A0 BD) decodes to U+FFFD
        data = vec![10, 0, 0, 8, 0, 1, b'k', 0, 3, 0xED, 0xA0, 0xBD, 0];
        assert_eq!(read_uncompressed(&data, BIG).unwrap().get_string("k"), "\u{FFFD}");
        // a partial two-byte character at the end of the string
        data = vec![10, 0, 0, 8, 0, 1, b'k', 0, 1, 0xC3, 0];
        assert!(read_uncompressed(&data, BIG).is_err());
    }

    fn nested(depth: usize) -> Compound {
        // a root compound whose chain of compounds below it makes `depth` levels in all
        let mut inner = Compound::new();
        for _ in 1..depth {
            let mut outer = Compound::new();
            outer.insert("c", Tag::Compound(inner));
            inner = outer;
        }
        inner
    }

    /// Runs a test body on a thread with a big stack: the test's own trees (built, cloned, encoded and dropped recursively) are deep.
    fn big_stack(f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new().stack_size(64 << 20).spawn(f).unwrap().join().unwrap();
    }

    #[test]
    fn depth_512_is_accepted_and_513_refused() {
        big_stack(depth_test);
    }

    fn depth_test() {
        assert!(read_uncompressed(&root(&nested(512)), BIG).is_ok());
        let e = read_uncompressed(&root(&nested(513)), BIG).unwrap_err();
        assert!(e.0.contains("depth > 512"), "{e:?}");
        // lists count too: a list of lists
        let mut t = Tag::List(3, vec![]);
        for _ in 0..511 {
            t = Tag::List(9, vec![t]);
        }
        assert!(read_uncompressed(&root(&comp(vec![("l", t.clone())])), BIG).is_err(), "512 lists below the root is 513 levels");
        let mut t = Tag::List(3, vec![]);
        for _ in 0..510 {
            t = Tag::List(9, vec![t]);
        }
        assert!(read_uncompressed(&root(&comp(vec![("l", t)])), BIG).is_ok());
    }

    #[test]
    fn root_must_be_a_named_compound() {
        assert_eq!(read_uncompressed(&[], BIG).unwrap_err().0, "Unexpected end of NBT data");
        assert_eq!(read_uncompressed(&[0], BIG).unwrap_err().0, "Root tag must be a named compound tag");
        assert_eq!(read_uncompressed(&[9, 0, 0], BIG).unwrap_err().0, "Root tag must be a named compound tag");
        // trailing data after the root is ignored
        let mut d = root(&Compound::new());
        d.extend_from_slice(&[1, 2, 3]);
        assert!(read_uncompressed(&d, BIG).is_ok());
        assert!(read_compressed(b"not gzip at all, no", BIG).is_err());
    }

    #[test]
    fn bad_lengths_are_errors_not_allocations() {
        // a byte array that claims 2 GB, one with a negative length, a list of ends with elements
        for body in [vec![7, 0, 1, b'a', 0x7F, 0xFF, 0xFF, 0xFF], vec![7, 0, 1, b'a', 0xFF, 0xFF, 0xFF, 0xFF], vec![11, 0, 1, b'a', 0x3F, 0xFF, 0xFF, 0xFF], vec![12, 0, 1, b'a', 0x1F, 0xFF, 0xFF, 0xFF]] {
            let mut d = vec![10, 0, 0];
            d.extend(body);
            d.push(0);
            assert!(read_uncompressed(&d, BIG).is_err());
        }
        let d = vec![10, 0, 0, 9, 0, 1, b'a', 0, 0, 0, 0, 5, 0];
        assert_eq!(read_uncompressed(&d, BIG).unwrap_err().0, "Missing type on ListTag");
        // a negative list length is an empty list
        let d = vec![10, 0, 0, 9, 0, 1, b'a', 3, 0xFF, 0xFF, 0xFF, 0xFF, 0];
        assert!(read_uncompressed(&d, BIG).unwrap().get("a").is_some());
        // an unknown tag id
        let d = vec![10, 0, 0, 13, 0, 1, b'a', 0];
        assert_eq!(read_uncompressed(&d, BIG).unwrap_err().0, "Invalid tag id: 13");
        // an empty list of an unknown type is fine, as in the game
        let d = vec![10, 0, 0, 9, 0, 1, b'a', 99, 0, 0, 0, 0, 0];
        assert!(read_uncompressed(&d, BIG).is_ok());
    }

    #[test]
    fn the_port_budget_stops_big_trees() {
        let c = comp(vec![("a", Tag::ByteArray(vec![0; 1000]))]);
        assert!(read_uncompressed(&root(&c), 500).is_err());
        assert!(read_uncompressed(&root(&c), 5000).is_ok());
        // gzip limit
        assert!(read_compressed(&gz(&c), 500).is_err());
    }

    #[test]
    fn review_the_budget_charges_what_a_tag_really_costs() {
        // a list of byte tags: every one is a `Tag` slot (72 bytes here) in a vector that may be twice as big as needed. The old charge was 32
        // a tag, so a million of them (72 MB of slots alone) were accepted at a budget of 33 MB.
        let list = |n: usize| {
            let mut d = vec![10, 0, 0, 9, 0, 1, b'l', 1];
            d.extend_from_slice(&(n as i32).to_be_bytes());
            d.extend(std::iter::repeat_n(0u8, n));
            d.push(0);
            d
        };
        let million = list(1_000_000);
        assert!(read_uncompressed(&million, 33 << 20).is_err());
        assert!(read_uncompressed(&million, 100 << 20).is_err(), "72 MB of slots, up to 144 MB with the growth of the vector");
        // a small list is fine under a budget that covers its slots
        let small = list(100_000);
        assert!(read_uncompressed(&small, 20 << 20).is_ok());
        assert!(read_uncompressed(&small, 5 << 20).is_err(), "100000 slots are 7.2 MB at least");
        // a compound entry costs its key twice (the entry and the index) and the `Entry` itself
        let names: Vec<String> = (0..2000).map(|i| format!("k{i:04}")).collect();
        let c = comp(names.iter().map(|k| (k.as_str(), Tag::Byte(1))).collect());
        let bytes = root(&c);
        assert!(read_uncompressed(&bytes, 100_000).is_err(), "2000 entries are at least 2000 * 104 bytes");
        assert!(read_uncompressed(&bytes, 4 << 20).is_ok());
    }

    #[test]
    fn truncation_at_every_byte_never_panics() {
        let c = comp(vec![("b", Tag::Byte(1)), ("st", s("h\u{e9}llo\u{1F600}")), ("li", Tag::List(10, vec![Tag::Compound(comp(vec![("q", Tag::Long(3))]))])), ("la", Tag::LongArray(vec![1, 2, 3])), ("ba", Tag::ByteArray(vec![1, 2, 3]))]);
        let full = root(&c);
        assert!(read_uncompressed(&full, BIG).is_ok());
        for n in 0..full.len() {
            assert!(read_uncompressed(&full[..n], BIG).is_err(), "cut at {n}");
        }
        let z = gz(&c);
        for n in 0..z.len() {
            assert!(read_compressed(&z[..n], BIG).is_err(), "gzip cut at {n}");
        }
    }

    #[test]
    fn getters_follow_the_game() {
        let c = comp(vec![
            ("long", Tag::Long(0x1_0000_0005)),
            ("neg", Tag::Float(-1.5)),
            ("big", Tag::Double(1e20)),
            ("nan", Tag::Double(f64::NAN)),
            ("byte", Tag::Byte(-3)),
            ("str", s("7")),
            ("ba", Tag::ByteArray(vec![9])),
            ("co", Tag::Compound(comp(vec![("k", Tag::Int(1))]))),
            ("lc", Tag::List(10, vec![Tag::Compound(Compound::new()), Tag::Compound(Compound::new())])),
            ("li", Tag::List(3, vec![Tag::Int(1)])),
            ("le", Tag::List(3, vec![])),
        ]);
        assert_eq!(c.get_int("long"), 5);
        assert_eq!(c.get_int("neg"), -2, "Mth.floor, not a plain cast");
        assert_eq!(c.get_int("big"), i32::MAX);
        assert_eq!(c.get_int("nan"), 0);
        assert_eq!(c.get_int("byte"), -3);
        assert_eq!(c.get_int("str"), 0, "a string is not a number");
        assert_eq!(c.get_int("missing"), 0);
        assert_eq!(c.get_string("str"), "7");
        assert_eq!(c.get_string("byte"), "");
        assert_eq!(c.get_byte_array("ba"), &[9]);
        assert!(c.get_byte_array("str").is_empty());
        assert!(c.get_long_array("ba").is_empty());
        assert_eq!(c.get_compound("co").get_int("k"), 1);
        assert!(c.get_compound("str").is_empty());
        assert_eq!(c.get_compound_list("lc").len(), 2);
        assert!(c.get_compound_list("li").is_empty());
        assert!(c.get_compound_list("le").is_empty());
        assert!(c.contains("str") && !c.contains("nope"));
    }

    #[test]
    fn a_repeated_key_keeps_its_first_place() {
        let mut c = Compound::new();
        c.insert("a", Tag::Int(1));
        c.insert("b", Tag::Int(2));
        c.insert("a", Tag::Int(3));
        assert_eq!(c.keys(), vec!["a", "b"]);
        assert_eq!(c.get_int("a"), 3);
        // the same in a file that repeats a key
        let d = vec![10, 0, 0, 1, 0, 1, b'a', 5, 1, 0, 1, b'a', 6, 0];
        assert_eq!(read_uncompressed(&d, BIG).unwrap().get_int("a"), 6);
    }

    #[test]
    fn string_hash_matches_java() {
        assert_eq!(java_string_hash(""), 0);
        assert_eq!(java_string_hash("a"), 97);
        assert_eq!(java_string_hash("ab"), 97 * 31 + 98);
        // "polygenelubricants".hashCode() is Integer.MIN_VALUE
        assert_eq!(java_string_hash("polygenelubricants"), i32::MIN);
        assert_eq!(java_string_hash("hello world"), 1794106052);
    }

    /// Hand-derived: bucket = (h ^ h>>>16) & 15 for a table of 16.
    /// "a"=97 -> 97&15=1, "b"=98 -> 2, "c"=99 -> 3, "A"=65 -> 1, "B"=66 -> 2, "Aa"=2112 -> 2112&15=0, "BB"=2112 -> 0.
    #[test]
    fn java_hash_map_order_small_table() {
        let mut c = Compound::new();
        for k in ["c", "b", "a"] {
            c.insert(k, Tag::Byte(0));
        }
        assert_eq!(c.keys_java_order(), vec!["a", "b", "c"], "buckets 1, 2, 3");
        // two keys in one bucket stay in file order: "Aa" and "BB" share a hash, "a" and "A" share bucket 1 (97 & 15 = 1, 65 & 15 = 1)
        let mut c = Compound::new();
        for k in ["a", "BB", "A", "Aa", "b"] {
            c.insert(k, Tag::Byte(0));
        }
        assert_eq!(c.keys_java_order(), vec!["BB", "Aa", "a", "A", "b"]);
    }

    #[test]
    fn java_hash_map_order_after_a_resize() {
        // 13 keys "k0".."k12" make the table 32 wide (13 > 12). hash("k" + d) = 107*31 + 48 + d = 3365 + d for one digit d (0..9);
        // for "k10", "k11", "k12": ((107*31 + 49)*31 + 48 + j) = 3366*31 + 48 + j = 104394 + j.
        // bucket = (h ^ h>>>16) & 31. 3365 = 0b110100100101 -> & 31 = 5, so k0..k9 are buckets 5..14. 104394 >> 16 = 1, so
        // (104394 ^ 1) & 31: 104394 & 31 = 10 (104394 = 3262*32 + 10), 10 ^ 1 = 11; "k11" -> 105 & 31... computed by the code below
        // against an independent evaluation of the rule, written the long way round.
        let keys: Vec<String> = (0..13).map(|i| format!("k{i}")).collect();
        let mut c = Compound::new();
        for k in &keys {
            c.insert(k, Tag::Byte(0));
        }
        let mut expected: Vec<(u32, usize, &String)> = keys
            .iter()
            .enumerate()
            .map(|(i, k)| {
                let mut h: u32 = 0;
                for u in k.encode_utf16() {
                    h = h.wrapping_mul(31).wrapping_add(u32::from(u));
                }
                (((h ^ (h >> 16)) & 31), i, k)
            })
            .collect();
        expected.sort();
        let want: Vec<&str> = expected.iter().map(|(_, _, k)| k.as_str()).collect();
        assert_eq!(c.keys_java_order(), want);
        // spot check by hand: k0 is bucket 5 and k9 bucket 14; k10 is 104394 -> bucket 11, which it shares with k6 (3365 + 6 = 3371 & 31 = 11):
        // k6 came first in the file, so k6 then k10
        let order = c.keys_java_order();
        let at = |k: &str| order.iter().position(|x| *x == k).unwrap();
        assert!(at("k0") < at("k1") && at("k5") < at("k6"));
        assert_eq!(at("k10"), at("k6") + 1);
        // with 12 keys the table is still 16 wide, so the order differs from a 32-wide table's
        let mut c12 = Compound::new();
        for k in &keys[..12] {
            c12.insert(k, Tag::Byte(0));
        }
        // 16 wide: k0 3365 & 15 = 5 ... k9 = 14, k10 = 104394 ^ 1 = ...: first k0
        assert_eq!(c12.keys_java_order()[0], "k0");
    }
}
