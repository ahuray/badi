//! Spelling correction in the languages the user enables
//! (`spelling_languages`). English keeps the embedded lexicon in `writing.rs`;
//! every other language uses its installed Hunspell dictionary, which only
//! proposes candidates: one candidate is offered as it is, several go to the
//! model, which must answer with one of them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::writing::CorrectionTarget;

/// Where system dictionary packages install `<name>.aff` and `<name>.dic`.
pub const DICTIONARY_DIRECTORIES: [&str; 2] = ["/usr/share/hunspell", "/usr/share/myspell/dicts"];
/// A correction keeps at most this many dictionary candidates.
const MAX_CANDIDATES: usize = 3;
/// Context the model sees when it chooses between candidates.
const CHOICE_CONTEXT_SCALARS: usize = 80;

/// Languages whose spelling Badi can correct: the European languages small
/// local models write well, each with an Arch dictionary package.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum SpellingLanguage {
    #[serde(rename = "en")]
    English,
    #[serde(rename = "de")]
    German,
    #[serde(rename = "fr")]
    French,
    #[serde(rename = "es")]
    Spanish,
    #[serde(rename = "it")]
    Italian,
    #[serde(rename = "nl")]
    Dutch,
    #[serde(rename = "pl")]
    Polish,
}

impl SpellingLanguage {
    pub const ALL: [Self; 7] = [
        Self::English,
        Self::German,
        Self::French,
        Self::Spanish,
        Self::Italian,
        Self::Dutch,
        Self::Polish,
    ];

    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::German => "de",
            Self::French => "fr",
            Self::Spanish => "es",
            Self::Italian => "it",
            Self::Dutch => "nl",
            Self::Polish => "pl",
        }
    }

    /// The language of a tag's primary subtag, such as `de-AT`.
    #[must_use]
    pub fn from_tag(tag: &str) -> Option<Self> {
        let primary = tag.split(['-', '_']).next()?.to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|language| language.tag() == primary)
    }

    fn from_detected(language: whatlang::Lang) -> Option<Self> {
        Some(match language {
            whatlang::Lang::Eng => Self::English,
            whatlang::Lang::Deu => Self::German,
            whatlang::Lang::Fra => Self::French,
            whatlang::Lang::Spa => Self::Spanish,
            whatlang::Lang::Ita => Self::Italian,
            whatlang::Lang::Nld => Self::Dutch,
            whatlang::Lang::Pol => Self::Polish,
            _ => return None,
        })
    }

    /// Dictionary names to try, most common first.
    #[must_use]
    pub const fn dictionaries(self) -> &'static [&'static str] {
        match self {
            Self::English => &["en_US", "en_GB"],
            Self::German => &["de_DE", "de_AT", "de_CH"],
            Self::French => &["fr_FR", "fr"],
            Self::Spanish => &["es_ES", "es_ANY", "es_MX"],
            Self::Italian => &["it_IT"],
            Self::Dutch => &["nl_NL"],
            Self::Polish => &["pl_PL"],
        }
    }

    /// German capitalizes nouns, so a capital mid-sentence is not a name.
    const fn capitalizes_nouns(self) -> bool {
        matches!(self, Self::German)
    }
}

/// What a request's text needs, before any candidate is computed.
pub(crate) enum Located<'a> {
    /// The English lexicon path in `writing.rs`.
    English(CorrectionTarget<'a>),
    /// A word the language's dictionary does not know.
    Dictionary {
        language: SpellingLanguage,
        target: CorrectionTarget<'a>,
    },
}

/// The enabled languages and their loaded dictionaries. The broker keeps one
/// for the process ([`global`]); the engine enables languages from settings.
pub struct Spelling {
    directories: Vec<PathBuf>,
    state: RwLock<State>,
}

struct State {
    generation: u64,
    enabled: Vec<SpellingLanguage>,
    dictionaries: BTreeMap<SpellingLanguage, Arc<spellbook::Dictionary>>,
}

impl Spelling {
    #[must_use]
    pub fn new(directories: Vec<PathBuf>) -> Self {
        Self {
            directories,
            state: RwLock::new(State {
                generation: 0,
                enabled: vec![SpellingLanguage::English],
                dictionaries: BTreeMap::new(),
            }),
        }
    }

    /// Enables exactly `languages` at once and returns the generation that
    /// [`Spelling::load`] completes. Dictionaries no longer needed are
    /// dropped; a language whose dictionary is not loaded yet has no
    /// corrections until then.
    pub fn enable(&self, languages: &[SpellingLanguage]) -> u64 {
        let Ok(mut state) = self.state.write() else {
            return 0;
        };
        state.generation += 1;
        state.enabled = languages.to_vec();
        state
            .dictionaries
            .retain(|language, _| languages.contains(language));
        state.generation
    }

    /// Loads the installed dictionaries the enabled languages need, off the
    /// broker's runtime, and keeps them only while `generation` is current.
    /// English needs none; a missing dictionary leaves its language without
    /// corrections.
    pub fn load(&self, generation: u64) {
        let missing: Vec<_> = match self.state.read() {
            Ok(state) if state.generation == generation => state
                .enabled
                .iter()
                .copied()
                .filter(|language| {
                    *language != SpellingLanguage::English
                        && !state.dictionaries.contains_key(language)
                })
                .collect(),
            _ => return,
        };
        for language in missing {
            let Some(dictionary) = self.read_dictionary(language) else {
                continue;
            };
            let Ok(mut state) = self.state.write() else {
                return;
            };
            if state.generation != generation {
                return;
            }
            state.dictionaries.insert(language, Arc::new(dictionary));
        }
    }

    fn read_dictionary(&self, language: SpellingLanguage) -> Option<spellbook::Dictionary> {
        let (aff, dic) = installed_dictionary(&self.directories, language)?;
        let aff = std::fs::read_to_string(aff).ok()?;
        let dic = std::fs::read_to_string(dic).ok()?;
        // Spellbook reads UTF-8 only; another encoding would misparse.
        if !aff.lines().any(|line| line.trim() == "SET UTF-8") {
            return None;
        }
        spellbook::Dictionary::new(&aff, &dic).ok()
    }

    /// The enabled language to correct `before` in: a confident detection
    /// decides, so another language's text is never corrected with this
    /// one's words; otherwise the field's language hint.
    fn language_for(&self, before: &str, hint: Option<&str>) -> Option<SpellingLanguage> {
        let enabled = self.state.read().ok()?.enabled.clone();
        let language = match crate::language::confident_language(before) {
            Some(detected) => SpellingLanguage::from_detected(detected)?,
            None => SpellingLanguage::from_tag(hint?)?,
        };
        enabled.contains(&language).then_some(language)
    }

    /// The misspelled word at the end of `before`, if any. A word without its
    /// trailing space may still be typed, so only an explicit request checks
    /// it. Cheap: no candidates are computed.
    pub(crate) fn locate<'a>(
        &self,
        before: &'a str,
        hint: Option<&str>,
        explicit: bool,
    ) -> Option<Located<'a>> {
        let language = self.language_for(before, hint)?;
        if language == SpellingLanguage::English {
            return crate::writing::correction_target(before).map(Located::English);
        }
        let target = dictionary_target(before, language)?;
        if target.delimiter.is_empty() && !explicit {
            return None;
        }
        let dictionary = self.dictionary(language)?;
        (!dictionary.check(target.word)).then_some(Located::Dictionary { language, target })
    }

    /// The dictionary's candidates for a located word: one word of letters,
    /// the same first letter, at most two edits away, in dictionary order.
    pub(crate) fn candidates(&self, language: SpellingLanguage, word: &str) -> Vec<String> {
        let Some(dictionary) = self.dictionary(language) else {
            return Vec::new();
        };
        let mut suggestions = Vec::new();
        dictionary.suggest(word, &mut suggestions);
        let mut candidates: Vec<String> = Vec::new();
        for suggestion in suggestions {
            if plausible_candidate(word, &suggestion) && !candidates.contains(&suggestion) {
                candidates.push(suggestion);
            }
            if candidates.len() == MAX_CANDIDATES {
                break;
            }
        }
        candidates
    }

    fn dictionary(&self, language: SpellingLanguage) -> Option<Arc<spellbook::Dictionary>> {
        self.state.read().ok()?.dictionaries.get(&language).cloned()
    }
}

static SPELLING: LazyLock<Spelling> = LazyLock::new(|| Spelling::new(dictionary_directories()));

/// The user's own dictionaries (`$XDG_DATA_HOME/hunspell`) first, then the
/// system packages'.
#[must_use]
pub fn dictionary_directories() -> Vec<PathBuf> {
    let user = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .map(|data| data.join("hunspell"));
    user.into_iter()
        .chain(DICTIONARY_DIRECTORIES.iter().map(PathBuf::from))
        .collect()
}

/// The process's spelling state, shared by the engine and the provider.
#[must_use]
pub fn global() -> &'static Spelling {
    &SPELLING
}

/// The installed `.aff` and `.dic` for `language` in `directories`.
#[must_use]
pub fn installed_dictionary(
    directories: &[PathBuf],
    language: SpellingLanguage,
) -> Option<(PathBuf, PathBuf)> {
    language.dictionaries().iter().find_map(|name| {
        directories.iter().find_map(|directory| {
            let aff = directory.join(format!("{name}.aff"));
            let dic = directory.join(format!("{name}.dic"));
            (is_file(&aff) && is_file(&dic)).then_some((aff, dic))
        })
    })
}

fn is_file(path: &Path) -> bool {
    path.metadata().is_ok_and(|metadata| metadata.is_file())
}

/// Letters of a cased script, the shape every spelling validator accepts.
#[must_use]
pub fn spelling_letter(character: char) -> bool {
    character.is_lowercase() || character.is_uppercase()
}

/// The last word of `before`, with its one trailing space, when it is all
/// letters, starts the field or follows whitespace, and is not an acronym or
/// (outside German) a capitalized word mid-sentence, which is likely a name.
fn dictionary_target(before: &str, language: SpellingLanguage) -> Option<CorrectionTarget<'_>> {
    let (context, delimiter) = before
        .strip_suffix(' ')
        .map_or((before, ""), |context| (context, " "));
    let start = context
        .char_indices()
        .rev()
        .take_while(|(_, character)| spelling_letter(*character))
        .last()?
        .0;
    let word = &context[start..];
    let preceding = context[..start].chars().next_back();
    let length = word.chars().count();
    if !(2..=32).contains(&length)
        || !preceding.is_none_or(char::is_whitespace)
        || word.chars().all(char::is_uppercase)
    {
        return None;
    }
    let capitalized = word.chars().next().is_some_and(char::is_uppercase);
    let sentence_start = context[..start]
        .trim_end()
        .chars()
        .next_back()
        .is_none_or(|character| ".!?:\n".contains(character));
    if capitalized && !sentence_start && !language.capitalizes_nouns() {
        return None;
    }
    Some(CorrectionTarget {
        word,
        suffix: &before[start..],
        delimiter,
    })
}

fn plausible_candidate(word: &str, candidate: &str) -> bool {
    let first = |text: &str| {
        text.chars()
            .next()
            .map(|character| character.to_lowercase().to_string())
    };
    candidate != word
        && (2..=32).contains(&candidate.chars().count())
        && candidate.chars().all(spelling_letter)
        && first(word) == first(candidate)
        && edit_distance(word, candidate) <= 2
}

/// Optimal string alignment distance over characters.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut rows = vec![vec![0; b.len() + 1]; a.len() + 1];
    for (index, row) in rows.iter_mut().enumerate() {
        row[0] = index;
    }
    for (index, cell) in rows[0].iter_mut().enumerate() {
        *cell = index;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (rows[i - 1][j] + 1)
                .min(rows[i][j - 1] + 1)
                .min(rows[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(rows[i - 2][j - 2] + 1);
            }
            rows[i][j] = best;
        }
    }
    rows[a.len()][b.len()]
}

/// The model chooses between dictionary candidates in the sentence's context.
/// The prompt stays short: it is never cached, so its prefill is the cost.
/// It ends inside an opened quote, so a base model names a word on the same
/// line; after a bare `Answer:` Gemma 3 1B starts a new line and never
/// answered (2026-10-19). The answer is read raw up to its closing quote
/// ([`chosen`]); echoing the typo is how the model abstains.
pub(crate) fn choice_payload(before: &str, word: &str, candidates: &[String]) -> Value {
    let skip = before
        .chars()
        .count()
        .saturating_sub(CHOICE_CONTEXT_SCALARS);
    let context: String = before.chars().skip(skip).collect();
    json!({
        "prompt": format!(
            "{}\nIn this sentence, \"{word}\" is a typo for one of: {}.\nThe intended word is \"",
            context.trim(),
            candidates.join(", ")
        ),
        "n_predict": 6,
        "temperature": 0.0,
        "seed": 42,
        "stop": ["\n"],
        "stream": true,
        "cache_prompt": false,
    })
}

/// The candidate the model named, if its answer, up to a closing quote, is
/// exactly one of them.
pub(crate) fn chosen<'a>(output: &str, candidates: &'a [String]) -> Option<&'a String> {
    let answer = output.split('"').next().unwrap_or_default();
    let answer = answer.trim().trim_end_matches(['.', ',', ';', '!', '?']);
    candidates
        .iter()
        .find(|candidate| candidate.as_str() == answer)
}

#[cfg(test)]
mod tests;
