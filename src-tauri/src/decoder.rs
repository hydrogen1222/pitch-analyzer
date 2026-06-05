// FCPE 解码器: latent (B=1, T, 360) -> f0 (T,), confidence (T,)
//
// 复刻 torchfcpe latent2cents_local_decoder:
//   1. 对每帧找 argmax, 取 [argmax-4, argmax+4] 共 9 个 cent_table 与 latent 加权平均得到 cents
//   2. f0 = 10 * 2^(cents / 1200)
//   3. confidence = max(latent) per frame
//   4. threshold 由上层 mask (设 f0=0)

use ndarray::ArrayView2;

pub struct FCPEDecoder {
    pub cent_table: Vec<f32>,
    pub out_dims: usize,
}

impl FCPEDecoder {
    pub fn new(cent_table: Vec<f32>) -> Self {
        let out_dims = cent_table.len();
        Self { cent_table, out_dims }
    }

    /// latent: (T, out_dims), 返回 (f0, confidence) 每个长度 T
    pub fn decode(&self, latent: ArrayView2<f32>, threshold: f32) -> (Vec<f32>, Vec<f32>) {
        let n_frames = latent.nrows();
        let mut f0 = Vec::with_capacity(n_frames);
        let mut conf = Vec::with_capacity(n_frames);

        for t in 0..n_frames {
            let row = latent.row(t);
            // argmax
            let mut max_val = f32::NEG_INFINITY;
            let mut max_idx = 0usize;
            for k in 0..self.out_dims {
                let v = row[k];
                if v > max_val {
                    max_val = v;
                    max_idx = k;
                }
            }
            conf.push(max_val);

            if max_val <= threshold {
                f0.push(0.0);
                continue;
            }

            // local window: [argmax-4, argmax+4], clamp 到 [0, out_dims-1]
            let mut num = 0.0f32;
            let mut den = 0.0f32;
            let start = max_idx as isize - 4;
            for j in 0..9 {
                let raw = start + j;
                let idx = raw.clamp(0, self.out_dims as isize - 1) as usize;
                let w = row[idx];
                num += self.cent_table[idx] * w;
                den += w;
            }
            let cents = if den > 0.0 { num / den } else { 0.0 };
            let f = 10.0 * (cents / 1200.0).exp2();
            f0.push(f);
        }

        (f0, conf)
    }
}
