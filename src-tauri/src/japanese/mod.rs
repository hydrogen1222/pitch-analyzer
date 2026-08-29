// 日语文本层: surface → reading span → mora → phoneme
//
// 与歌词显示 (LyricToken) 和音高层 (NoteEvent) 解耦:
//   - 显示 token 可以覆盖多个 mora (位 → く・ら・い)
//   - 一个 mora 可以对应多个 NoteEvent (转音)
//   - 多个 mora 可以共享一个持续 NoteEvent
//
// provider 优先级 (任务书 §2.4):
//   用户 override > ruby/furigana (未支持) > UniDic 词典 > kana-only > heuristic

pub mod mora;
pub mod phoneme;
pub mod reading;

pub use mora::parse_kana_moras;
pub use reading::{JapaneseReadingProvider, KanaOnlyProvider};
