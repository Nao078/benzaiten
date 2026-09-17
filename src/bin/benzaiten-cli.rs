use benzaiten::{
    domain::{
        lyrics::{apply_readings, parse_lyrics},
        project::{Project, ReadingSource},
    },
    jobs::{run_sync, ToolSettings},
    lrc::writer,
    project::storage,
    pronunciation::{english::EnglishPronunciationEngine, PronunciationEngine},
};
use std::path::PathBuf;

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() < 5 {
        return Err(
            "Usage: benzaiten-cli <audio> <lyrics.txt> <ffmpeg> <wav2vec2.onnx> <output.lrc> [--language auto|en|jp] [--vocabulary <tokenizer.json>] [--generate-reading | --reading <katakana.txt>]"
                .into(),
        );
    }
    let mut language = "en".to_owned();
    let mut vocabulary = None;
    let mut reading_path = None;
    let mut generate_reading = false;
    let mut index = 5;
    while index < args.len() {
        match args[index].to_string_lossy().as_ref() {
            "--language" if index + 1 < args.len() => {
                language = args[index + 1].to_string_lossy().into_owned();
                index += 2;
            }
            "--vocabulary" if index + 1 < args.len() => {
                vocabulary = Some(PathBuf::from(&args[index + 1]));
                index += 2;
            }
            "--reading" if index + 1 < args.len() => {
                reading_path = Some(PathBuf::from(&args[index + 1]));
                index += 2;
            }
            "--generate-reading" => {
                generate_reading = true;
                index += 1;
            }
            option => return Err(format!("unknown or incomplete option: {option}")),
        }
    }
    if generate_reading && reading_path.is_some() {
        return Err("--generate-reading and --reading cannot be used together".into());
    }
    let audio = std::fs::canonicalize(&args[0]).map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(&args[1]).map_err(|e| e.to_string())?;
    let settings = ToolSettings {
        ffmpeg: args[2].clone().into(),
        model: args[3].clone().into(),
        vocabulary,
        language,
        threads: 8,
    };
    let mut lyrics = parse_lyrics(&text);
    if let Some(path) = reading_path {
        let readings = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        apply_readings(&mut lyrics, &readings)?;
    } else if generate_reading {
        let engine = EnglishPronunciationEngine;
        for line in &mut lyrics {
            if line.original_text.trim().is_empty() {
                continue;
            }
            let result = engine.generate(&line.original_text)?;
            if !result.unknown_words.is_empty() {
                eprintln!(
                    "dictionary fallback on line {}: {}",
                    line.id + 1,
                    result.unknown_words.join(", ")
                );
            }
            line.reading_text = Some(result.reading);
            line.reading_source = Some(ReadingSource::Generated);
        }
    }
    let lyrics = run_sync(&audio, &lyrics, &settings)?;
    let project = Project {
        audio_path: audio,
        lyrics,
        ..Default::default()
    };
    let output = PathBuf::from(&args[4]);
    // Retain unresolved rows for manual correction even when LRC export fails.
    storage::save(&output.with_extension("json"), &project)?;
    writer::export(&output, &project)
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
