// Mandarin reading provider abstraction (任务书 §9.2)

/// 单字/词组读音质量 (§9.3):
/// Contextual = 词组词典消歧; Dictionary = 单字表 (最常见读音);
/// Ambiguous = 已知多音字但上下文无法确定; Unknown = 无读音依据。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ReadingQuality {
    Contextual,
    Dictionary,
    #[default]
    Ambiguous,
    Unknown,
}

/// 一个汉字音节的读音分析结果。
/// char 区间以 primary_text 的 char 下标为基准。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MandarinSyllableSpan {
    pub surface: String,
    pub char_start: usize,
    pub char_end: usize,
    /// 带调拼音 (如 "wo3"); 未知读音为空串
    pub pinyin: String,
    /// 声调 1-5 (5 = 轻声); 仅用于 G2P 消歧与诊断, 不是旋律约束 (§9.4)
    pub tone: Option<u8>,
    /// initial/final 音素序列 (读音未知时为空)
    pub phonemes: Vec<String>,
    pub quality: ReadingQuality,
}

/// 普通话读音分析器抽象。实现必须无锁可共享 (Send + Sync)。
pub trait MandarinReadingProvider: Send + Sync {
    fn name(&self) -> &'static str;

    /// 把一行歌词原文解析为音节 span 序列。
    /// 非汉字片段 (拉丁/数字/标点) 不产生音节, 由调用方做显示层处理。
    fn analyze(&self, text: &str) -> Result<Vec<MandarinSyllableSpan>, String>;
}

use serde::{Deserialize, Serialize};

/// 保守词典实现 (任务书 Commit 7):
///   phrase/contextual dictionary > single-character fallback (§9.3);
///   词典未覆盖的汉字 → quality = Ambiguous, pinyin 为空 (不猜)。
pub struct DictMandarinReadingProvider;

/// 词组词典: 优先于单字表, 用于多音字消歧与常见歌词词。
/// pinyin 用 tone-number 记法 (wo3 / de5 轻声)。
const PHRASE_PINYIN: &[(&str, &[&str])] = &[
    ("重新", &["chong2", "xin1"]),
    ("音乐", &["yin1", "yue4"]),
    ("快乐", &["kuai4", "le4"]),
    ("觉得", &["jue2", "de5"]),
    ("我们", &["wo3", "men5"]),
    ("你们", &["ni3", "men5"]),
    ("他们", &["ta1", "men5"]),
    ("她们", &["ta1", "men5"]),
    ("什么", &["shen2", "me5"]),
    ("怎么", &["zen3", "me5"]),
    ("因为", &["yin1", "wei4"]),
    ("所以", &["suo3", "yi3"]),
    ("还是", &["hai2", "shi4"]),
    ("还有", &["hai2", "you3"]),
    ("回来", &["hui2", "lai2"]),
    ("再见", &["zai4", "jian4"]),
    ("也许", &["ye3", "xu3"]),
    ("世界", &["shi4", "jie4"]),
    ("未来", &["wei4", "lai2"]),
    ("现在", &["xian4", "zai4"]),
    ("时间", &["shi2", "jian1"]),
    ("希望", &["xi1", "wang4"]),
    ("喜欢", &["xi3", "huan5"]),
    ("幸福", &["xing4", "fu2"]),
    ("伤心", &["shang1", "xin1"]),
    ("开心", &["kai1", "xin1"]),
    ("眼泪", &["yan3", "lei4"]),
    ("微笑", &["wei1", "xiao4"]),
    ("美丽", &["mei3", "li4"]),
    ("自由", &["zi4", "you2"]),
    ("梦想", &["meng4", "xiang3"]),
    ("回忆", &["hui2", "yi4"]),
    ("孤单", &["gu1", "dan1"]),
    ("寂寞", &["ji4", "mo4"]),
    ("温柔", &["wen1", "rou2"]),
    ("永远", &["yong3", "yuan3"]),
    ("明天", &["ming2", "tian1"]),
    ("昨天", &["zuo2", "tian1"]),
    ("今天", &["jin1", "tian1"]),
    ("后来", &["hou4", "lai2"]),
    ("开始", &["kai1", "shi3"]),
    ("结束", &["jie2", "shu4"]),
    ("长大", &["zhang3", "da4"]),
    ("成长", &["cheng2", "zhang3"]),
    ("长大", &["zhang3", "da4"]),
];

/// 多音字表: 单字 fallback (最常见读音) + 是否多音字标记。
/// 多音字且词组词典未命中 → 读最常见音但 quality = Ambiguous (§9.3)。
const CHAR_PINYIN: &[(&str, &str, bool)] = &[
    ("我", "wo3", false),
    ("你", "ni3", false),
    ("他", "ta1", false),
    ("她", "ta1", false),
    ("它", "ta1", false),
    ("们", "men5", false),
    ("吗", "ma5", false),
    ("呢", "ne5", false),
    ("吧", "ba5", false),
    ("啊", "a5", false),
    ("呀", "ya5", false),
    ("的", "de5", false),
    ("了", "le5", true),
    ("是", "shi4", false),
    ("在", "zai4", false),
    ("有", "you3", false),
    ("不", "bu4", false),
    ("没", "mei2", false),
    ("很", "hen3", false),
    ("太", "tai4", false),
    ("都", "dou1", true),
    ("也", "ye3", false),
    ("就", "jiu4", false),
    ("要", "yao4", true),
    ("能", "neng2", false),
    ("会", "hui4", true),
    ("想", "xiang3", false),
    ("知", "zhi1", false),
    ("道", "dao4", false),
    ("说", "shuo1", false),
    ("话", "hua4", false),
    ("去", "qu4", false),
    ("来", "lai2", false),
    ("过", "guo4", true),
    ("看", "kan4", false),
    ("听", "ting1", false),
    ("读", "du2", false),
    ("写", "xie3", false),
    ("学", "xue2", false),
    ("做", "zuo4", false),
    ("吃", "chi1", false),
    ("喝", "he1", true),
    ("打", "da3", true),
    ("开", "kai1", false),
    ("关", "guan1", false),
    ("头", "tou2", false),
    ("后", "hou4", false),
    ("里", "li3", false),
    ("外", "wai4", false),
    ("中", "zhong1", true),
    ("上", "shang4", false),
    ("下", "xia4", false),
    ("大", "da4", false),
    ("小", "xiao3", false),
    ("多", "duo1", false),
    ("少", "shao3", true),
    ("高", "gao1", false),
    ("低", "di1", false),
    ("长", "chang2", true),
    ("短", "duan3", false),
    ("天", "tian1", false),
    ("地", "di4", true),
    ("山", "shan1", false),
    ("水", "shui3", false),
    ("火", "huo3", false),
    ("土", "tu3", false),
    ("木", "mu4", false),
    ("人", "ren2", false),
    ("口", "kou3", false),
    ("日", "ri4", false),
    ("月", "yue4", false),
    ("风", "feng1", false),
    ("云", "yun2", false),
    ("雨", "yu3", false),
    ("雪", "xue3", false),
    ("花", "hua1", false),
    ("草", "cao3", false),
    ("鸟", "niao3", false),
    ("虫", "chong2", false),
    ("鱼", "yu2", false),
    ("爸", "ba4", false),
    ("妈", "ma1", false),
    ("儿", "er2", false),
    ("女", "nv3", false),
    ("子", "zi5", true),
    ("王", "wang2", false),
    ("朋", "peng2", false),
    ("友", "you3", false),
    ("同", "tong2", true),
    ("老", "lao3", false),
    ("师", "shi1", false),
    ("心", "xin1", false),
    ("手", "shou3", false),
    ("眼", "yan3", false),
    ("真", "zhen1", false),
    ("美", "mei3", false),
    ("新", "xin1", false),
    ("旧", "jiu4", false),
    ("快", "kuai4", false),
    ("慢", "man4", false),
    ("爱", "ai4", false),
    ("恨", "hen4", false),
    ("笑", "xiao4", false),
    ("哭", "ku1", false),
    ("飞", "fei1", false),
    ("走", "zou3", false),
    ("跑", "pao3", false),
    ("坐", "zuo4", false),
    ("站", "zhan4", false),
    ("睡", "shui4", false),
    ("家", "jia1", false),
    ("门", "men2", false),
    ("路", "lu4", false),
    ("车", "che1", false),
    ("马", "ma3", false),
    ("狗", "gou3", false),
    ("猫", "mao1", false),
    ("春", "chun1", false),
    ("夏", "xia4", false),
    ("秋", "qiu1", false),
    ("冬", "dong1", false),
    ("早", "zao3", false),
    ("晚", "wan3", false),
    ("夜", "ye4", false),
    ("今", "jin1", false),
    ("明", "ming2", false),
    ("年", "nian2", false),
    ("时", "shi2", false),
    ("光", "guang1", false),
    ("声", "sheng1", false),
    ("梦", "meng4", false),
    ("海", "hai3", false),
    ("星", "xing1", false),
    ("空", "kong1", true),
    ("风", "feng1", false),
    ("灯", "deng1", false),
    ("歌", "ge1", false),
    ("唱", "chang4", false),
    ("曲", "qu3", true),
    ("诗", "shi1", false),
    ("情", "qing2", false),
    ("谁", "shui2", true),
    ("什", "shen2", true),
    ("还", "hai2", true),
    ("着", "zhe5", true),
    ("得", "de5", true),
    ("重", "zhong4", true),
    ("相", "xiang1", true),
    ("教", "jiao1", true),
    ("发", "fa1", true),
    ("乐", "le4", true),
    ("只", "zhi3", true),
    ("种", "zhong3", true),
    ("行", "xing2", true),
    ("应", "ying1", true),
    ("为", "wei4", true),
    ("和", "he2", true),
    ("见", "jian4", false),
    ("面", "mian4", false),
    ("梦", "meng4", false),
    ("泪", "lei4", false),
    ("声", "sheng1", false),
    ("影", "ying3", false),
    ("静", "jing4", false),
    ("夜", "ye4", false),
    ("黑", "hei1", false),
    ("白", "bai2", false),
    ("红", "hong2", false),
    ("蓝", "lan2", false),
    ("黄", "huang2", false),
    ("绿", "lv4", false),
    ("暖", "nuan3", false),
    ("冷", "leng3", false),
    ("热", "re4", false),
    ("远", "yuan3", false),
    ("近", "jin4", false),
    ("深", "shen1", false),
    ("浅", "qian3", false),
];

fn tone_of(pinyin: &str) -> Option<u8> {
    let digit = pinyin.chars().last()?;
    digit
        .to_digit(10)
        .map(|d| d as u8)
        .filter(|&d| (1..=5).contains(&d))
}

fn split_fragments(text: &str) -> Vec<(String, usize, usize, bool)> {
    // (text, char_start, char_end, is_han)
    let mut out: Vec<(String, usize, usize, bool)> = Vec::new();
    for (ci, c) in text.chars().enumerate() {
        let is_han =
            ('\u{4E00}'..='\u{9FFF}').contains(&c) || ('\u{3400}'..='\u{4DBF}').contains(&c);
        if let Some(last) = out.last_mut() {
            if last.3 == is_han {
                last.0.push(c);
                last.2 = ci + 1;
                continue;
            }
        }
        out.push((c.to_string(), ci, ci + 1, is_han));
    }
    out
}

impl MandarinReadingProvider for DictMandarinReadingProvider {
    fn name(&self) -> &'static str {
        "dict-mandarin-g2p"
    }

    fn analyze(&self, text: &str) -> Result<Vec<MandarinSyllableSpan>, String> {
        let chars: Vec<char> = text.chars().collect();
        let mut spans = Vec::new();

        for (frag, cs, _ce, is_han) in split_fragments(text) {
            if !is_han {
                continue; // 拉丁/数字/标点不产生音节
            }
            let frag_len = frag.chars().count();
            // 1. 词组词典: 滑窗匹配 (长优先)
            let mut consumed = 0usize; // 已被词组覆盖的 char 数 (相对 cs)
            while consumed < frag_len {
                let mut matched = false;
                for (phrase, pinyins) in PHRASE_PINYIN {
                    let plen = phrase.chars().count();
                    if plen <= 1 || consumed + plen > frag_len {
                        continue;
                    }
                    let candidate: String =
                        chars[cs + consumed..cs + consumed + plen].iter().collect();
                    if candidate == *phrase {
                        for (i, py) in pinyins.iter().enumerate() {
                            spans.push(MandarinSyllableSpan {
                                surface: chars[cs + consumed + i].to_string(),
                                char_start: cs + consumed + i,
                                char_end: cs + consumed + i + 1,
                                pinyin: py.to_string(),
                                tone: tone_of(py),
                                phonemes: crate::mandarin::phoneme::pinyin_to_phonemes(py),
                                quality: ReadingQuality::Contextual,
                            });
                        }
                        consumed += plen;
                        matched = true;
                        break;
                    }
                }
                if matched {
                    continue;
                }
                // 2. 单字 fallback
                let ch = chars[cs + consumed];
                let ch_str = ch.to_string();
                let entry = CHAR_PINYIN.iter().find(|(c, _, _)| *c == ch_str);
                match entry {
                    Some((_, pinyin, is_polyphone)) => {
                        // 多音字没有词组上下文 → Ambiguous (给最常见音, 明确标注)
                        let quality = if *is_polyphone {
                            ReadingQuality::Ambiguous
                        } else {
                            ReadingQuality::Dictionary
                        };
                        spans.push(MandarinSyllableSpan {
                            surface: ch_str,
                            char_start: cs + consumed,
                            char_end: cs + consumed + 1,
                            pinyin: pinyin.to_string(),
                            tone: tone_of(pinyin),
                            phonemes: crate::mandarin::phoneme::pinyin_to_phonemes(pinyin),
                            quality,
                        });
                    }
                    None => {
                        // 词典未覆盖: 音节结构存在 (1 字 = 1 音节), 读音 Unknown
                        spans.push(MandarinSyllableSpan {
                            surface: ch_str,
                            char_start: cs + consumed,
                            char_end: cs + consumed + 1,
                            pinyin: String::new(),
                            tone: None,
                            phonemes: Vec::new(),
                            quality: ReadingQuality::Unknown,
                        });
                    }
                }
                consumed += 1;
            }
        }
        Ok(spans)
    }
}
