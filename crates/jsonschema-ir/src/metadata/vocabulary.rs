use string_interner::symbol::SymbolU32;

pub struct VocabularyId(u32);

struct Vocabulary {
    name: SymbolU32,
    enabled: bool,
}
