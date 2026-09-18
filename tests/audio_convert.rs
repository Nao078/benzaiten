//! 可逆音源（WAV）からFLAC/MP3への再エンコード（`audio::convert`）を、
//! 実際に生成したPCMデータで検証する。デコードして中身を読み返すところ
//! まで行うことで、エンコーダが出力する構造的に壊れたファイル
//! （例：`flacenc`が末尾の端数ブロックを残したときにSymphoniaが読めなく
//! なる既知の癖）を見逃さないようにしている。

use std::f32::consts::TAU;

/// 1秒・440Hz・ステレオのサイン波WAVを書き出す。
fn make_test_wav(path: &std::path::Path) {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 44_100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for i in 0..44_100 {
        let t = i as f32 / 44_100.0;
        let value = (t * 440.0 * TAU).sin();
        let sample = (value * i16::MAX as f32 * 0.5) as i16;
        writer.write_sample(sample).unwrap();
        writer.write_sample(sample).unwrap();
    }
    writer.finalize().unwrap();
}

#[test]
fn convert_wav_to_flac_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let wav_path = dir.path().join("src.wav");
    make_test_wav(&wav_path);

    let flac_path = dir.path().join("out.flac");
    benzaiten::audio::convert::to_flac(&wav_path, &flac_path).unwrap();
    assert!(std::fs::metadata(&flac_path).unwrap().len() > 100);

    // 変換後のファイルを実際にデコードし直せることを確認する。ここが
    // 通らないと、書き出したFLACをアプリ自身（再生・再アライメント）で
    // 二度と開けないことになる。
    let decoded = benzaiten::audio::preprocess::decode(&flac_path, None).unwrap();
    assert_eq!(decoded.channels, 2);
    assert_eq!(decoded.sample_rate, 44_100);
    // 末尾の無音パディング（1ブロック未満）を許容し、元のフレーム数以上
    // であることを確認する。
    assert!(decoded.interleaved.len() >= 88_200);
}

#[test]
fn convert_wav_to_mp3_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let wav_path = dir.path().join("src.wav");
    make_test_wav(&wav_path);

    let mp3_path = dir.path().join("out.mp3");
    benzaiten::audio::convert::to_mp3(&wav_path, &mp3_path).unwrap();
    assert!(std::fs::metadata(&mp3_path).unwrap().len() > 100);

    let decoded = benzaiten::audio::preprocess::decode(&mp3_path, None).unwrap();
    assert_eq!(decoded.channels, 2);
    assert_eq!(decoded.sample_rate, 44_100);
    // MP3のエンコーダ遅延分だけ長くなるので、多少の増加は許容する。
    assert!(decoded.interleaved.len() >= 88_200);
}
