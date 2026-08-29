use serde::{Deserialize, Serialize};

/// 三级音高数据中的 "Clean Pitch Track"
/// 保留真实连续 F0 曲线用于绘图，另存逐帧 RMS 与离散 NoteEvent 供字幕使用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PitchTrack {
    pub times: Vec<f32>,
    pub frequencies: Vec<f32>,
    pub confidences: Vec<f32>,
    pub midis: Vec<f32>,
    /// 逐帧(10ms) RMS 包络，与 times 对齐，供歌词字级对齐使用
    #[serde(default)]
    pub rms: Vec<f32>,
    /// 离散稳定音符事件 (Annotation Note Track)
    #[serde(default)]
    pub note_events: Vec<NoteEvent>,
    /// 逐帧 log-mel 谱通量 (相邻帧 L1 距离)，音素/音节边界的声学证据。
    /// 与 times 对齐；旧工程文件缺省为空。
    #[serde(default)]
    pub flux: Vec<f32>,
}

/// 离散稳定音符事件 (Annotation Note Track)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteEvent {
    pub start: f32,
    pub end: f32,
    pub midi: i32,
    pub note_name: String,
    pub confidence: f32,
}

impl NoteEvent {
    pub fn duration(&self) -> f32 {
        (self.end - self.start).max(0.0)
    }
}

/// Note Tracker 参数集中管理，避免散落 magic numbers
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct NoteTrackingParams {
    /// 新音成为有效音符所需的最短时长
    pub min_note_duration_ms: f32,
    /// 候选新音需持续该时长才承认切换 (debounce)
    pub switch_confirm_ms: f32,
    /// 短于该时长且恰差一个八度 → 视为 octave error
    pub octave_error_max_ms: f32,
    /// 半音边界 hysteresis (cents)，避免 vibrato 触发闪烁
    pub semitone_hysteresis_cents: f32,
    /// 绑定时忽略 token 首尾的极短 margin
    pub edge_ignore_ms: f32,
}

impl Default for NoteTrackingParams {
    fn default() -> Self {
        Self {
            min_note_duration_ms: 45.0,
            switch_confirm_ms: 60.0,
            octave_error_max_ms: 80.0,
            semitone_hysteresis_cents: 25.0,
            edge_ignore_ms: 20.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AnalyzerConfig {
    pub confidence_threshold: f32,
    pub fmin: f32,
    pub fmax: f32,
    pub smoothing: usize,
    pub median_smoothing: usize,
    pub quantize: bool,
    pub note_tracking: NoteTrackingParams,
}

impl Default for AnalyzerConfig {
    fn default() -> Self {
        Self {
            confidence_threshold: 0.3,
            fmin: 50.0,
            fmax: 2000.0,
            smoothing: 15,
            median_smoothing: 11,
            quantize: false,
            note_tracking: NoteTrackingParams::default(),
        }
    }
}

/// 前端下发/后端保存的当前分析参数 (全局只使用同一套参数)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisParams {
    pub confidence_threshold: f32,
    pub fmin: f32,
    pub fmax: f32,
    pub smoothing: f64,
    pub median_smoothing: f64,
    pub quantize: bool,
    pub min_note_duration_ms: f32,
}

impl Default for AnalysisParams {
    fn default() -> Self {
        Self {
            confidence_threshold: 0.3,
            fmin: 65.0,
            fmax: 1300.0,
            smoothing: 15.0,
            median_smoothing: 11.0,
            quantize: false,
            min_note_duration_ms: 45.0,
        }
    }
}

impl AnalysisParams {
    /// 转为 analyzer 配置
    pub fn to_analyzer_config(&self) -> AnalyzerConfig {
        let mut note_tracking = NoteTrackingParams::default();
        note_tracking.min_note_duration_ms = self.min_note_duration_ms;
        AnalyzerConfig {
            confidence_threshold: self.confidence_threshold,
            fmin: self.fmin,
            fmax: self.fmax,
            smoothing: self.smoothing as usize,
            median_smoothing: self.median_smoothing as usize,
            quantize: self.quantize,
            note_tracking,
        }
    }
}

/// 一个字内的一个检测音 (由 NoteEvent 或帧级分段生成)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PitchNote {
    pub start_time: f32,
    pub end_time: f32,
    pub median_midi: f32,
    pub mean_midi: f32,
    pub rounded_midi: i32,
    pub confidence_mean: f32,
    pub point_count: usize,
}

/// 一个字在句内的时间范围与音高绑定结果
/// 对齐结果的来源 (优先级从高到低: EnhancedLrc > ForcedAlign > MoraDp > WeightedFallback)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlignmentSource {
    /// enhanced LRC 自带的逐字时间锚点
    EnhancedLrc,
    /// 音素级 forced alignment (P3, 尚未实现)
    ForcedAlign,
    /// 莫拉感知声学 DP (复用 mel 特征)
    MoraDp,
    /// 加权均匀分配 (最后兜底)
    WeightedFallback,
}

/// 一个 token 没有音高 badge 的原因 (区分"物理无音高"与"算法丢失")
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnpitchedReason {
    /// 该时间窗内完全没有发声帧
    NoVoicing,
    /// 清音无声化 / 促音闭塞等: 有语言单位但物理上无稳定 F0
    ClosureOrDevoicing,
    /// 有发声但置信度低于阈值
    LowPitchConfidence,
    /// 时间窗存在但与任何 NoteEvent 都没有足够重叠
    NoOverlappingNote,
    /// 该行从未完成时间对齐
    AlignmentMissing,
}

/// token 与 NoteEvent 的一次软绑定 (many-to-many: 一个 token 可绑多个音, 多个 token 可共享一个音)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteBinding {
    pub note_event_index: usize,
    pub overlap_ms: f32,
    /// 重叠占 token 窗口的比例
    pub overlap_ratio_token: f32,
    /// 重叠占 NoteEvent 的比例
    pub overlap_ratio_note: f32,
    /// 综合评分
    pub score: f32,
}

/// 一段原文及其读音 (P1: UniDic 词典可用时由 LinderaUnidicProvider 填充;
/// kana-only provider 中假名片段 reading=surface, 汉字片段 reading 为空表示读音未知)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadingSpan {
    pub surface: String,
    /// 规范化假名读音; 空串 = 读音未知 (需词典/override)
    pub reading: String,
    /// 发音 (若词典提供), 否则与 reading 相同
    pub pronunciation: String,
    /// 原文中的 char 下标区间 [start, end)
    pub char_start: usize,
    pub char_end: usize,
    /// 允许覆盖多个显示字符 (熟字训/当て字没有逐字拆分的唯一答案)
    pub display_start: usize,
    pub display_end: usize,
    /// 在行 mora 序列中的区间 [start, end)
    pub mora_start: usize,
    pub mora_end: usize,
    pub confidence: f32,
}

/// 一个莫拉 (拍)。时间与音高绑定在对齐阶段回填。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoraUnit {
    pub kana: String,
    pub phonemes: Vec<String>,
    /// 所属 ReadingSpan 在 line.reading_spans 中的下标
    pub reading_span_id: usize,
    /// 原文 char 区间
    pub char_start: usize,
    pub char_end: usize,
    #[serde(default)]
    pub start_time: Option<f32>,
    #[serde(default)]
    pub end_time: Option<f32>,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub note_bindings: Vec<NoteBinding>,
}

/// 一个字内没有音高时, 不再渲染为无解释的空白
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Unpitched {
    pub reason: UnpitchedReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LyricToken {
    pub text: String,
    pub start_time: Option<f32>,
    pub end_time: Option<f32>,
    #[serde(default)]
    pub pitch_notes: Vec<PitchNote>,
    /// 字幕默认显示的主音 (紧凑 UI 的代表音; 完整信息在 pitch_notes)
    #[serde(default)]
    pub primary_note: Option<PitchNote>,
    /// 原文 char 区间 [start, end) (显示 token 可能覆盖多个 char)
    #[serde(default)]
    pub char_start: usize,
    #[serde(default)]
    pub char_end: usize,
    /// 所属 ReadingSpan 下标 (line.reading_spans)
    #[serde(default)]
    pub reading_span_ids: Vec<usize>,
    #[serde(default)]
    pub alignment_confidence: f32,
    #[serde(default)]
    pub alignment_source: Option<AlignmentSource>,
    /// 无音高时的原因 (Some = 确定无音高及其成因)
    #[serde(default)]
    pub unpitched_reason: Option<UnpitchedReason>,
}

impl LyricToken {
    /// 新 token: 其余字段在对齐/绑定阶段回填
    pub fn new(text: String, char_start: usize, char_end: usize) -> Self {
        Self {
            text,
            start_time: None,
            end_time: None,
            pitch_notes: Vec::new(),
            primary_note: None,
            char_start,
            char_end,
            reading_span_ids: Vec::new(),
            alignment_confidence: 0.0,
            alignment_source: None,
            unpitched_reason: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LyricLine {
    /// Display text (含翻译, 用 " | " 分隔)
    pub text: String,
    pub start_time: Option<f32>,
    pub end_time: Option<f32>,
    pub tokens: Vec<LyricToken>,
    #[serde(default)]
    pub primary_text: String,
    #[serde(default)]
    pub translations: Vec<String>,
    /// 逐字时间是否为程序自动估计 (DP 对齐或均匀分配)。
    /// 自动估计的时间在重新分析音轨后需要重新对齐；
    /// enhanced LRC 自带的逐字时间 (false) 则永远保留。
    #[serde(default)]
    pub token_timing_auto: bool,
    /// 原文片段 → 读音 (P1 UniDic 之前: 假名片段自带读音, 汉字片段未知)
    #[serde(default)]
    pub reading_spans: Vec<ReadingSpan>,
    /// 行内莫拉序列 (对齐的最小文本单位)
    #[serde(default)]
    pub moras: Vec<MoraUnit>,
}

impl LyricLine {
    /// 新行: tokens/reading_spans/moras 由解析器随后填充
    pub fn new(
        text: String,
        primary_text: String,
        translations: Vec<String>,
        start_time: Option<f32>,
        end_time: Option<f32>,
    ) -> Self {
        Self {
            text,
            start_time,
            end_time,
            tokens: Vec::new(),
            primary_text,
            translations,
            token_timing_auto: false,
            reading_spans: Vec::new(),
            moras: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectData {
    pub audio_path: Option<String>,
    pub pitch_track: Option<PitchTrack>,
    pub lyrics: Vec<LyricLine>,
    #[serde(default)]
    pub analysis_params: Option<AnalysisParams>,
}

/// MIDI → 音名 (C4 / F#4 ...)，NaN → "---"
pub fn midi_to_note_name(midi: f32) -> String {
    let names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    if midi.is_nan() {
        return "---".to_string();
    }
    let m = midi.round() as i32;
    format!("{}{}", names[m.rem_euclid(12) as usize], m.div_euclid(12) - 1)
}
