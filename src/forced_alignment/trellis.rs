//! CTC（Connectionist Temporal Classification）のトレリス上でViterbi
//! アルゴリズムを実行し、正解トークン列を音響フレームへ強制的に整列させる。

/// 1トークンに割り当てられたフレーム区間と、その区間内での平均確信度。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TokenSpan {
    pub start_frame: usize,
    pub end_frame: usize,
    pub confidence: f32,
}

/// トレースバッファ（`frame数 × state数`）の上限。極端に長い音声・歌詞の
/// 組み合わせでメモリを使い果たさないための安全弁。
const MAX_TRACE_CELLS: usize = 64_000_000;

/// 標準的なCTCのトポロジ（blank, token, blank, token, ..., blank）に
/// 沿ったViterbiアラインメント。
///
/// 状態数は`2 * トークン数 + 1`で、偶数番目の状態がblank、奇数番目の
/// 状態が`tokens[state / 2]`に対応する。各フレームで取りうる遷移は
/// 「留まる」「1つ先の状態へ進む」「2つ先の状態へジャンプ（直前と
/// 異なるトークン同士の間のblankを飛ばす）」の3通りで、これは同じ
/// トークンが連続する場合にblankを挟まないと区別できないというCTCの
/// 制約に対応している。各フレームで最も対数尤度の高い経路を
/// 動的計画法で求め、`trace`にどの遷移を選んだかを記録しておいて
/// 最後に終端から先頭へ逆向きにたどることでトークンごとのフレーム
/// 区間を復元する。
///
/// 加えて、1秒あたりに進んでよい状態数に上限を設ける（詳細は関数内の
/// コメント）。`frame_duration_ms`はその上限をフレーム数へ換算するために
/// 使う。
pub fn force_align(
    emissions: &[Vec<f32>],
    tokens: &[usize],
    blank: usize,
    frame_duration_ms: f64,
) -> Result<Vec<TokenSpan>, String> {
    if emissions.is_empty() || tokens.is_empty() {
        return Err("forced alignment requires emissions and transcript tokens".to_owned());
    }
    if !frame_duration_ms.is_finite() || frame_duration_ms <= 0.0 {
        return Err("forced-alignment frame duration must be positive".to_owned());
    }
    let vocabulary = emissions[0].len();
    if vocabulary == 0 || emissions.iter().any(|frame| frame.len() != vocabulary) {
        return Err("acoustic emissions have inconsistent vocabulary dimensions".to_owned());
    }
    if blank >= vocabulary || tokens.iter().any(|token| *token >= vocabulary) {
        return Err("transcript token is outside the acoustic-model vocabulary".to_owned());
    }

    let states = tokens
        .len()
        .checked_mul(2)
        .and_then(|n| n.checked_add(1))
        .ok_or("forced-alignment transcript is too long")?;
    let cells = emissions
        .len()
        .checked_mul(states)
        .ok_or("forced-alignment input is too long")?;
    if cells > MAX_TRACE_CELLS {
        return Err(format!(
            "forced alignment needs {cells} trace cells (limit {MAX_TRACE_CELLS})"
        ));
    }
    if emissions.len() < tokens.len() {
        return Err("audio has fewer acoustic frames than transcript tokens".to_owned());
    }

    // 経路が実際の発話より早く進みすぎるのを防ぐ上限（詳細は
    // `run_viterbi`のコメントを参照）。人間が歌える速さには物理的な
    // 上限があるが、音響的根拠があいまいな音源（歌唱特有の発声、
    // エフェクトのかかった声など）では、その上限内では経路が
    // 見つからないことも起こり得る。そのため、まず厳しめの上限で
    // 試し、経路が見つからなければ上限を緩めて（最終的には無制限で）
    // 再試行する。これにより、通常は“あり得ない先読み”を防ぎつつ、
    // 難しい音源でも整列そのものに失敗するという後退は起こさない。
    const MAX_CHARACTERS_PER_SECOND: f64 = 16.0;
    let attempts = [
        Some(MAX_CHARACTERS_PER_SECOND),
        Some(MAX_CHARACTERS_PER_SECOND * 4.0),
        None,
    ];
    let mut last_error = String::new();
    for max_characters_per_second in attempts {
        match run_viterbi(
            emissions,
            tokens,
            blank,
            frame_duration_ms,
            states,
            cells,
            max_characters_per_second,
        ) {
            Ok(spans) => return Ok(spans),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

/// CTC ViterbiのDP本体。`max_characters_per_second`が`Some`なら、
/// 経路が実際の発話より早く進みすぎるのを防ぐ上限を課す。歌詞中に
/// 同一・類似フレーズが近くに複数回登場すると、その区間の確信度が
/// あいまいなために、Viterbi経路が実際より早いタイミングへ
/// “先読み”して複数行分を一気に消費してしまうことがある（後で
/// 帳尻合わせとして別の場所に不自然に長い停留が生じる）。
///
/// この上限は「曲の先頭から現在までの平均ペース」ではなく、
/// 「直近`PACE_WINDOW_SECONDS`秒間だけで進んだ状態数」で判定する
/// （スライディングウィンドウ）。曲の先頭からの累積ペースで判定すると、
/// 長いイントロや、実際の平均ペースが上限よりゆっくりな曲では、
/// ペースに余裕がある区間で“貯金”ができてしまい、その貯金を後半の
/// どこか1箇所で一気に使い切れてしまうため、上限が実質的に無効化
/// されてしまう。直近の窓だけで判定することで、曲のどの位置でも
/// 「物理的にあり得ない速さでの先読み」を一貫して禁止できる。
/// 「留まる」方向には一切制限をかけないため、長いイントロ・間奏の
/// 許容度はこれまでどおり保たれる。`max_characters_per_second`が
/// `None`の場合は上限なし（元のアルゴリズムと同じ）。
#[allow(clippy::too_many_arguments)]
fn run_viterbi(
    emissions: &[Vec<f32>],
    tokens: &[usize],
    blank: usize,
    frame_duration_ms: f64,
    states: usize,
    cells: usize,
    max_characters_per_second: Option<f64>,
) -> Result<Vec<TokenSpan>, String> {
    const STATES_PER_CHARACTER: f64 = 2.0;
    const PACE_SAFETY_MARGIN: f64 = 1.15;
    const PACE_WINDOW_SECONDS: f64 = 1.0;
    let frames_total = emissions.len();
    let pace_limit = max_characters_per_second.map(|max_characters_per_second| {
        let duration_seconds = frames_total as f64 * frame_duration_ms / 1000.0;
        let required_pace_per_second = states as f64 / duration_seconds;
        let max_states_per_second = (max_characters_per_second * STATES_PER_CHARACTER)
            .max(required_pace_per_second * PACE_SAFETY_MARGIN);
        let max_states_per_frame = max_states_per_second * frame_duration_ms / 1000.0;
        let window_frames =
            ((PACE_WINDOW_SECONDS * 1000.0 / frame_duration_ms).round() as usize).max(1);
        let max_states_per_window = (max_states_per_second * PACE_WINDOW_SECONDS).round() as usize;
        (max_states_per_frame, window_frames, max_states_per_window)
    });

    // `previous`/`current`は各状態までの最良経路の対数尤度（フレームごとに
    // ローリングして更新）。`trace`は各(フレーム, 状態)セルでどの遷移
    // （0=留まる、1=1つ進む、2=2つジャンプ）を選んだかを記録し、
    // 後で経路を逆にたどるために使う。`frontier_history`は各フレーム
    // 時点で最も尤度が高かった状態（argmax）の履歴（スライディング
    // ウィンドウの上限を求めるために使う）。
    //
    // 注意：フロンティアを「有限な尤度を持つ最遠の状態」（単に
    // `is_finite()`）で定義してはいけない。CTCのblankを含め、どの
    // ラベルの確率もソフトマックス上は厳密には0にならないため、
    // 音響的根拠が実質無くても「留まる」「1つ進む」を繰り返すだけで
    // 機械的に毎フレーム状態が“到達可能”になってしまう。その結果、
    // 上限（クロック）が実際の音響的根拠と無関係に回り続け、曲の
    // 平均ペースが上限よりゆっくりな曲では時間経過とともに上限が
    // 実質無制限に近づいてしまい、制限として機能しなくなる。
    // 「その時点で本当に最も尤度が高い状態」（argmax）を基準にすれば、
    // 音響的根拠が実際にその状態を裏付けている場合にだけ前進したと
    // みなされる。
    let negative_infinity = f32::NEG_INFINITY;
    let mut previous = vec![negative_infinity; states];
    let mut current = vec![negative_infinity; states];
    let mut trace = vec![0_u8; cells];
    let mut frontier_history: Vec<usize> = Vec::with_capacity(frames_total);
    previous[0] = 0.0;

    for (frame_index, logits) in emissions.iter().enumerate() {
        let log_norm = log_sum_exp(logits);
        current.fill(negative_infinity);
        let pace_ceiling = match pace_limit {
            None => states - 1,
            Some((max_states_per_frame, window_frames, max_states_per_window)) => {
                if frame_index < window_frames {
                    // 直近`window_frames`分の履歴がまだ無い曲の冒頭では、
                    // 曲の先頭からの経過時間ベースで代用する（このごく
                    // 短い期間では“貯金”が問題になるほど蓄積しようが
                    // ないため）。
                    ((max_states_per_frame * (frame_index + 1) as f64).round() as usize)
                        .min(states - 1)
                } else {
                    (frontier_history[frame_index - window_frames] + max_states_per_window)
                        .min(states - 1)
                }
            }
        };
        let mut frontier = 0_usize;
        let mut frontier_score = negative_infinity;
        for state in 0..=pace_ceiling {
            let label = state_label(state, tokens, blank);
            let emission = logits[label] - log_norm;
            let mut best = previous[state];
            let mut predecessor = 0_u8;
            if state >= 1 && previous[state - 1] > best {
                best = previous[state - 1];
                predecessor = 1;
            }
            // 2つ先へのジャンプは、間のblankを省略できる場合
            // （直前のトークンと今のトークンが異なる場合）のみ許可する。
            if state >= 2
                && state % 2 == 1
                && tokens[state / 2] != tokens[state / 2 - 1]
                && previous[state - 2] > best
            {
                best = previous[state - 2];
                predecessor = 2;
            }
            if best.is_finite() {
                current[state] = best + emission;
                trace[frame_index * states + state] = predecessor;
                // 同点（無地の区間など、証拠が完全に無差別な場合）は、
                // より先の状態を優先する。真に劣っている状態には決して
                // 進まない（`>`のみが弱くなるケース）一方、strictな`>`
                // のままだと無地の区間でフロンティアが硬直し、曲の終端
                // 状態へ一生到達できず整列に失敗してしまう。
                if current[state] >= frontier_score {
                    frontier_score = current[state];
                    frontier = state;
                }
            }
        }
        frontier_history.push(frontier);
        std::mem::swap(&mut previous, &mut current);
    }

    // 終端は「最後のトークン状態」か「その後のblank」のどちらでもよいので、
    // 尤度の高い方を選ぶ。
    let last_token_state = states - 2;
    let mut state = if previous[states - 1] > previous[last_token_state] {
        states - 1
    } else {
        last_token_state
    };
    if !previous[state].is_finite() {
        return Err("could not find a CTC path through the complete lyrics".to_owned());
    }

    // 記録しておいた`trace`を終端から先頭へたどりながら、奇数状態
    // （＝トークン状態）を通過するたびにそのトークンの開始・終了フレームを
    // 更新し、確信度（そのフレームでの正規化済み事後確率）を積算する。
    let mut starts = vec![usize::MAX; tokens.len()];
    let mut ends = vec![0; tokens.len()];
    let mut confidence_sum = vec![0.0_f32; tokens.len()];
    let mut confidence_count = vec![0_usize; tokens.len()];
    for frame in (0..emissions.len()).rev() {
        if state % 2 == 1 {
            let token_index = state / 2;
            starts[token_index] = starts[token_index].min(frame);
            ends[token_index] = ends[token_index].max(frame + 1);
            let logits = &emissions[frame];
            confidence_sum[token_index] +=
                (logits[tokens[token_index]] - log_sum_exp(logits)).exp();
            confidence_count[token_index] += 1;
        }
        let predecessor = trace[frame * states + state] as usize;
        state = state.saturating_sub(predecessor);
    }

    starts
        .into_iter()
        .zip(ends)
        .zip(confidence_sum.into_iter().zip(confidence_count))
        .enumerate()
        .map(|(index, ((start_frame, end_frame), (sum, count)))| {
            if start_frame == usize::MAX || count == 0 {
                return Err(format!("transcript token {index} was not aligned"));
            }
            Ok(TokenSpan {
                start_frame,
                end_frame,
                confidence: sum / count as f32,
            })
        })
        .collect()
}

/// トレリスの状態番号から、その状態が表すラベル（blankかトークンID）を
/// 求める。偶数状態はblank、奇数状態は`tokens[state / 2]`。
fn state_label(state: usize, tokens: &[usize], blank: usize) -> usize {
    if state.is_multiple_of(2) {
        blank
    } else {
        tokens[state / 2]
    }
}

/// 数値的に安定な形でのlogsumexp。フレームのlogitsをソフトマックス
/// 正規化する際の対数正規化定数（分配関数の対数）を求めるために使う。
fn log_sum_exp(values: &[f32]) -> f32 {
    let maximum = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    maximum
        + values
            .iter()
            .map(|value| (*value - maximum).exp())
            .sum::<f32>()
            .ln()
}

#[cfg(test)]
mod tests {
    use super::force_align;

    #[test]
    fn aligns_repeated_tokens_through_a_blank() {
        // blank=0, A=1。強い経路は「A, blank, A」のみ。
        let emissions = vec![
            vec![0.0, 8.0],
            vec![8.0, 0.0],
            vec![0.0, 8.0],
            vec![8.0, 0.0],
        ];
        let spans = force_align(&emissions, &[1, 1], 0, 20.0).unwrap();
        assert_eq!(spans[0].start_frame, 0);
        assert_eq!(spans[1].start_frame, 2);
    }

    #[test]
    fn rejects_non_positive_frame_duration() {
        let emissions = vec![vec![0.0, 8.0]];
        assert!(force_align(&emissions, &[1], 0, 0.0).is_err());
    }

    #[test]
    fn pace_limit_prevents_cramming_many_tokens_into_a_short_burst() {
        // 最初の4フレームだけ"A"の証拠を強く鳴らし、残り200フレームは
        // 無地（どのラベルも無差別）にする。ペース上限が無ければ、
        // 8個の"A"トークン（blankを挟むため計17状態）を証拠が強い
        // 最初の数フレームへ全部詰め込むのが最も尤度が高くなるが、
        // それは1秒あたり数十文字という現実にあり得ない速さになる。
        // ペース上限により、最後のトークンは（状態17全体に到達できる
        // ようになる）ずっと後のフレームまで開始できないはずである。
        let mut emissions = Vec::new();
        for _ in 0..4 {
            emissions.push(vec![0.0, 8.0]);
        }
        for _ in 0..200 {
            emissions.push(vec![0.0, 0.0]);
        }

        let spans = force_align(&emissions, &[1; 8], 0, 20.0).unwrap();
        assert!(
            spans.last().unwrap().start_frame >= 20,
            "last token started at frame {}, expected it to be spread out past the initial burst",
            spans.last().unwrap().start_frame
        );
    }

    #[test]
    fn a_long_quiet_lead_in_does_not_bank_slack_for_a_later_burst() {
        // 6秒間のblank優勢な“イントロ”の後、わずか5フレームだけ40個の
        // "A"トークン（blankを挟むため計81状態）ぶんの強い証拠を鳴らし、
        // その後は無地にする。もし上限が「曲の先頭からの累積ペース」で
        // 判定されていたら、6秒間ほぼ無進行だった分の“貯金”を使って、
        // このバーストだけで81状態ぜんぶに到達できてしまう
        // （＝実際に起きていた回帰）。直近1秒間だけを見るスライディング
        // ウィンドウなら、貯金は蓄積されず、バースト直後の時点では
        // まだ半分程度の状態にしか到達できないはずである。
        let mut emissions = Vec::new();
        for _ in 0..300 {
            emissions.push(vec![8.0, 0.0]); // 約6秒のblank優勢な無音区間
        }
        for _ in 0..5 {
            emissions.push(vec![0.0, 8.0]); // 短いバースト
        }
        for _ in 0..400 {
            emissions.push(vec![0.0, 0.0]); // 無地
        }

        let spans = force_align(&emissions, &[1; 40], 0, 20.0).unwrap();
        assert!(
            spans.last().unwrap().start_frame >= 340,
            "last token started at frame {}, expected the long quiet lead-in to grant no extra burst budget",
            spans.last().unwrap().start_frame
        );
    }
}
