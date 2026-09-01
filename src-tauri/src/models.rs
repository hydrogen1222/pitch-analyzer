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
    /// [legacy/兼容] NoteTracker 产出的事件 (旧工程字段, 保留读取;
    /// canonical musical notes 见 musical_notes)
    #[serde(default)]
    pub note_events: Vec<NoteEvent>,
    /// canonical musical note track (GAME 或 LegacyFcpeTracker 产出,
    /// 来源由 musical_note_source 明确标注, 不允许静默混淆)
    #[serde(default)]
    pub musical_notes: Vec<crate::note_engine::MusicalNoteEvent>,
    #[serde(default)]
    pub musical_note_source: crate::note_engine::MusicalNoteSource,
    /// 产生 canonical notes 的模型标识 (如 "GAME-1.0.3-small")
    #[serde(default)]
    pub musical_note_model: Option<String>,
    /// Raw GAME candidates retained for diagnostics; production binding uses
    /// the evidence-backed `musical_notes` track.
    #[serde(default)]
    pub raw_game_notes: Vec<crate::note_engine::MusicalNoteEvent>,
    /// Full evidence and decision data for the canonical note post-processor.
    #[serde(default)]
    pub canonical_sung_notes: Vec<CanonicalSungNote>,
    /// 逐帧 log-mel 谱通量 (相邻帧 L1 距离)，音素/音节边界的声学证据。
    /// 与 times 对齐；旧工程文件缺省为空。
    #[serde(default)]
    pub flux: Vec<f32>,
}

/// 音符内部的音高表现 (不属于新音符): 颤音 / 滑音 / 起音 / 收尾等
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PitchGestureKind {
    Stable,
    Vibrato,
    Glide,
    Scoop,
    Fall,
    Unvoiced,
    Uncertain,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PitchGesture {
    pub start: f32,
    pub end: f32,
    pub kind: PitchGestureKind,
    #[serde(default)]
    pub center_midi: Option<f32>,
    #[serde(default)]
    pub from_midi: Option<f32>,
    #[serde(default)]
    pub to_midi: Option<f32>,
    /// 颤音深度 (cents)
    #[serde(default)]
    pub depth_cents: Option<f32>,
    /// 颤音频率 (Hz)
    #[serde(default)]
    pub rate_hz: Option<f32>,
    #[serde(default)]
    pub confidence: f32,
}

/// 离散稳定音符事件 (Annotation Note Track)。
/// v2: 由连续 midi 空间的 stable-plateau 引擎生成, 不再由半音取整驱动;
///     center_midi 为连续中心, midi 仅是输出时的最近音名。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteEvent {
    pub start: f32,
    pub end: f32,
    pub midi: i32,
    pub note_name: String,
    pub confidence: f32,
    /// v2: 连续稳定中心 (未取整); 旧工程为 None
    #[serde(default)]
    pub center_midi: Option<f32>,
    /// v2: 稳定平台时长 (秒); 旧工程为 0
    #[serde(default)]
    pub stable_duration: f32,
    /// v2: 音内音高表现 (颤音/滑音等), 不构成新音符
    #[serde(default)]
    pub gestures: Vec<PitchGesture>,
    /// 生成器版本 (2 = stable-plateau engine)
    #[serde(default)]
    pub tracker_version: u32,
}

impl NoteEvent {
    pub fn duration(&self) -> f32 {
        (self.end - self.start).max(0.0)
    }
}

/// Note Tracker 参数集中管理，避免散落 magic numbers。
/// v1 字段保留作 serde 兼容 (部分已被 v2 取代)。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct NoteTrackingParams {
    /// 最短有效音符时长 (lyrics binder 的 primary 过滤仍使用)
    pub min_note_duration_ms: f32,
    /// [v1 遗留, v2 不再使用] 候选新音确认时长 (被 confirm_duration_ms 取代)
    #[serde(default)]
    pub switch_confirm_ms: f32,
    /// [v1 遗留, v2 不再使用] 短八度错误上限 (plateau 引擎自然处理)
    #[serde(default)]
    pub octave_error_max_ms: f32,
    /// [v1 遗留, v2 不再使用] 名义半音滞回 (v2 用连续 cents 滞回取代)
    #[serde(default)]
    pub semitone_hysteresis_cents: f32,
    /// 绑定时忽略 token 首尾的极短 margin
    #[serde(default)]
    pub edge_ignore_ms: f32,

    // ── v2 stable-plateau engine (连续 cents 空间) ──
    /// 稳定半径: 中心 ±stay 内绝不因取整变化换音
    #[serde(default = "default_stay_radius_cents")]
    pub stay_radius_cents: f32,
    /// 切换偏离: 超过此偏离才开始收集新目标候选
    #[serde(default = "default_switch_deviation_cents")]
    pub switch_deviation_cents: f32,
    /// 确认新目标所需的最小音程分离
    #[serde(default = "default_switch_separation_cents")]
    pub switch_separation_cents: f32,
    /// 常规新音确认时长
    #[serde(default = "default_confirm_duration_ms")]
    pub confirm_duration_ms: f32,
    /// 短倚音保留的最短时长 (配合高置信/明显音程)
    #[serde(default = "default_appoggiatura_min_ms")]
    pub appoggiatura_min_ms: f32,
    /// 候选平台的稳定度上限 (MAD, cents)
    #[serde(default = "default_candidate_max_mad_cents")]
    pub candidate_max_mad_cents: f32,
    /// 可桥接的无声间隙 (帧, 10ms/帧)
    #[serde(default = "default_bridge_gap_frames")]
    pub bridge_gap_frames: usize,
}

impl Default for NoteTrackingParams {
    fn default() -> Self {
        Self {
            min_note_duration_ms: 45.0,
            switch_confirm_ms: 60.0,
            octave_error_max_ms: 80.0,
            semitone_hysteresis_cents: 25.0,
            edge_ignore_ms: 20.0,
            stay_radius_cents: default_stay_radius_cents(),
            switch_deviation_cents: default_switch_deviation_cents(),
            switch_separation_cents: default_switch_separation_cents(),
            confirm_duration_ms: default_confirm_duration_ms(),
            appoggiatura_min_ms: default_appoggiatura_min_ms(),
            candidate_max_mad_cents: default_candidate_max_mad_cents(),
            bridge_gap_frames: default_bridge_gap_frames(),
        }
    }
}

fn default_stay_radius_cents() -> f32 {
    40.0
}
fn default_switch_deviation_cents() -> f32 {
    70.0
}
fn default_switch_separation_cents() -> f32 {
    80.0
}
fn default_confirm_duration_ms() -> f32 {
    70.0
}
fn default_appoggiatura_min_ms() -> f32 {
    45.0
}
fn default_candidate_max_mad_cents() -> f32 {
    25.0
}
fn default_bridge_gap_frames() -> usize {
    3
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
            smoothing: 21,
            median_smoothing: 13,
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
            smoothing: 21.0,
            median_smoothing: 13.0,
            quantize: false,
            min_note_duration_ms: 45.0,
        }
    }
}

impl AnalysisParams {
    /// 转为 analyzer 配置
    pub fn to_analyzer_config(&self) -> AnalyzerConfig {
        let note_tracking = NoteTrackingParams {
            min_note_duration_ms: self.min_note_duration_ms,
            ..Default::default()
        };
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
impl Default for PitchTrack {
    fn default() -> Self {
        Self {
            times: Vec::new(),
            frequencies: Vec::new(),
            confidences: Vec::new(),
            midis: Vec::new(),
            rms: Vec::new(),
            note_events: Vec::new(),
            flux: Vec::new(),
            musical_notes: Vec::new(),
            musical_note_source: Default::default(),
            musical_note_model: None,
            raw_game_notes: Vec::new(),
            canonical_sung_notes: Vec::new(),
        }
    }
}

/// 对齐结果的来源 (优先级从高到低: EnhancedLrc > ForcedAlign > MoraDp > WeightedFallback)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlignmentSource {
    /// enhanced LRC 自带的逐字时间锚点
    EnhancedLrc,
    /// 真实声学模型 forced alignment (MMS-FA 等)
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
    /// GAME and FCPE disagree beyond the evidence tolerance.
    LowCrossModelAgreement,
    /// GAME returned a region, but FCPE has insufficient voiced support.
    GameUnsupportedByFcpe,
    /// More than one note is plausible and no primary note is safe to show.
    AmbiguousMusicalNote,
    /// The lyric unit has no sufficiently reliable acoustic alignment.
    AlignmentLowConfidence,
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

/// Display-level group between language and acoustics. A group can contain
/// several display glyphs and a different number of moras (`二人` → `ふたり`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadingDisplayGroup {
    pub id: usize,
    pub char_start: usize,
    pub char_end: usize,
    pub reading_span_id: usize,
    pub mora_start: usize,
    pub mora_end: usize,
    pub surface: String,
    pub reading: String,
    /// true 表示这是由显式 `漢字(かな)` 拆出的假名音高槽位。
    /// char_* 仍指向源歌词汉字范围，但 UI/导出应把音高标在 reading 上。
    #[serde(default)]
    pub phonetic: bool,
    #[serde(default)]
    pub start_time: Option<f32>,
    #[serde(default)]
    pub end_time: Option<f32>,
    #[serde(default)]
    pub pitch_notes: Vec<PitchNote>,
    #[serde(default)]
    pub primary_note: Option<PitchNote>,
    #[serde(default)]
    pub unpitched_reason: Option<UnpitchedReason>,
}

/// 一个莫拉 (拍)。时间与音高绑定在对齐阶段回填。
///
/// display 映射规则 (任务书 5.3): kana-only 路径 reading == surface, char 区间精确;
/// 词典读音 (reading != surface) 时 char 区间为整个 ReadingSpan 的 surface span
/// (coarse) —— 熟字训/多字词不存在"这个 mora 对应哪个汉字"的唯一答案, 不伪造映射。
/// 精确定位用 reading_offset_* (读音文本内的下标)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoraUnit {
    pub kana: String,
    pub phonemes: Vec<String>,
    /// 所属 ReadingSpan 在 line.reading_spans 中的下标
    pub reading_span_id: usize,
    /// 原文 char 区间 (kana-only: 精确; 词典读音: 粗粒度 = 整个 span 的 surface 区间)
    pub char_start: usize,
    pub char_end: usize,
    /// 在 span 读音文本内的 char 区间 (始终精确)
    #[serde(default)]
    pub reading_offset_start: usize,
    #[serde(default)]
    pub reading_offset_end: usize,
    /// surface 上的显示区间 (可覆盖多个字)
    #[serde(default)]
    pub display_start: usize,
    #[serde(default)]
    pub display_end: usize,
    /// 所属显示组；同一组内的 mora 不会被伪造为每个 glyph 的独立声学真值。
    #[serde(default)]
    pub display_group_id: usize,
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
    /// 所属 display group 下标。
    #[serde(default)]
    pub reading_group_ids: Vec<usize>,
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
            reading_group_ids: Vec::new(),
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
    /// `漢字(かな)` annotations. Coordinates are always display-text chars.
    #[serde(default)]
    pub ruby_annotations: Vec<crate::japanese::reading::RubyAnnotation>,
    /// 逐字时间是否为程序自动估计 (DP 对齐或均匀分配)。
    /// 自动估计的时间在重新分析音轨后需要重新对齐；
    /// enhanced LRC 自带的逐字时间 (false) 则永远保留。
    #[serde(default)]
    pub token_timing_auto: bool,
    /// 原文片段 → 读音 (P1 UniDic 之前: 假名片段自带读音, 汉字片段未知)
    #[serde(default)]
    pub reading_spans: Vec<ReadingSpan>,
    #[serde(default)]
    pub reading_display_groups: Vec<ReadingDisplayGroup>,
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
            ruby_annotations: Vec::new(),
            token_timing_auto: false,
            reading_spans: Vec::new(),
            reading_display_groups: Vec::new(),
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

/// Independent FCPE evidence for one GAME region.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteEvidence {
    pub fcpe_frame_count: usize,
    pub voiced_coverage: f32,
    pub median_midi: Option<f32>,
    pub midi_mad_cents: Option<f32>,
    pub pitch_delta_cents: Option<f32>,
    pub support_score: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SungNoteClass {
    Stable,
    Ornament,
    /// Reserved for a future contour-based glide classifier. A normal change
    /// from one sung note to another must remain Stable.
    Transition,
    Uncertain,
}

/// Evidence-backed canonical note used by compact/detail display and binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalSungNote {
    pub event_ids: Vec<u32>,
    pub start: f32,
    pub end: f32,
    pub game_midi: f32,
    pub fcpe_median_midi: Option<f32>,
    pub display_midi: f32,
    pub voiced_coverage: f32,
    pub fcpe_support: f32,
    pub confidence: f32,
    pub class: SungNoteClass,
    pub evidence: NoteEvidence,
}

/// MIDI → 音名 (C4 / F#4 ...)，NaN → "---"
pub fn midi_to_note_name(midi: f32) -> String {
    let names = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    if midi.is_nan() {
        return "---".to_string();
    }
    let m = midi.round() as i32;
    format!(
        "{}{}",
        names[m.rem_euclid(12) as usize],
        m.div_euclid(12) - 1
    )
}
