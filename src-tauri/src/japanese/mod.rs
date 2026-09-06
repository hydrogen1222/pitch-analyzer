// 日语文本层: surface → reading span → mora → phoneme
//
// 与歌词显示 (LyricToken) 和音高层 (NoteEvent) 解耦:
//   - 显示 token 可以覆盖多个 mora (位 → く・ら・い)
//   - 一个 mora 可以对应多个 NoteEvent (转音)
//   - 多个 mora 可以共享一个持续 NoteEvent
//
// provider 优先级 (任务书 §2.4):
//   用户 override > ruby/furigana > UniDic 词典 > kana-only > heuristic

pub mod mora;
pub mod phoneme;
pub mod pipeline;
pub mod pitch_policy;
pub mod reading;
pub mod unidic;

pub use mora::parse_kana_moras;
pub use phoneme::{JapanesePhonemizer, PhonemeSpec, RuleBasedPhonemizer};
pub use pipeline::build_japanese_layers;
pub use pitch_policy::{
    classify_phoneme_class, classify_unit_kind, nucleus_phoneme_range, phoneme_pitch_bearing,
    pitch_bearing_policy,
};
pub use reading::{
    parse_ruby_text, ruby_display_offset, JapaneseReadingProvider, KanaOnlyProvider,
    ParsedRubyText, RubyAnnotation, RubyParseError,
};
pub use unidic::LinderaUnidicProvider;
