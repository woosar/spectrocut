use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use anyhow::{Context, Result};

use super::settings::Settings;
use crate::helpers::{full_command, parse_progress_line};

enum JobEvent {
    Progress(String, String),
    Log(String),
}

/// A running ffmpeg process with its parsed `-progress` output.
pub struct RenderJob {
    child: Child,
    rx: Receiver<JobEvent>,
    started: Instant,
    pub total_secs: Option<f64>,
    pub out_secs: f64,
    pub fps: String,
    pub speed: String,
    pub log: VecDeque<String>,
    pub finished: Option<Result<(), String>>,
}

impl RenderJob {
    pub fn spawn(settings: &Settings, total_secs: Option<f64>) -> Result<Self> {
        let (program, mut args) = full_command(settings);
        // Insert run-only flags right after the `ffmpeg` word.
        let at = args.iter().position(|a| a == "ffmpeg").map_or(0, |i| i + 1);
        let extra = ["-hide_banner", "-nostats", "-progress", "pipe:1", "-y"];
        args.splice(at..at, extra.iter().map(|s| s.to_string()));

        let mut child = Command::new(&program)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("could not start {program}"))?;

        let (tx, rx) = mpsc::channel();
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let tx_out = tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some((k, v)) = parse_progress_line(&line) {
                    let _ = tx_out.send(JobEvent::Progress(k.into(), v.into()));
                }
            }
        });
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let _ = tx.send(JobEvent::Log(line));
            }
        });

        Ok(Self {
            child,
            rx,
            started: Instant::now(),
            total_secs,
            out_secs: 0.0,
            fps: "-".into(),
            speed: "-".into(),
            log: VecDeque::new(),
            finished: None,
        })
    }

    /// Drain pending output and check whether ffmpeg has exited.
    pub fn poll(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                JobEvent::Progress(k, v) => match k.as_str() {
                    "out_time_us" | "out_time_ms" => {
                        // Both keys are microseconds in practice.
                        if let Ok(us) = v.parse::<f64>() {
                            self.out_secs = us / 1_000_000.0;
                        }
                    }
                    "fps" => self.fps = v,
                    "speed" => self.speed = v,
                    _ => {}
                },
                JobEvent::Log(line) => {
                    if !line.trim().is_empty() {
                        self.log.push_back(line);
                        if self.log.len() > 200 {
                            self.log.pop_front();
                        }
                    }
                }
            }
        }
        if self.finished.is_none()
            && let Ok(Some(status)) = self.child.try_wait()
        {
            self.finished = Some(if status.success() {
                Ok(())
            } else {
                Err(format!("ffmpeg exited with {status}"))
            });
        }
    }

    pub fn ratio(&self) -> f64 {
        match self.total_secs {
            Some(t) if t > 0.0 => (self.out_secs / t).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }

    pub fn elapsed_secs(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    pub fn eta_secs(&self) -> Option<f64> {
        let r = self.ratio();
        (r > 0.01).then(|| self.elapsed_secs() * (1.0 - r) / r)
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.finished = Some(Err("cancelled".into()));
    }
}

impl Drop for RenderJob {
    fn drop(&mut self) {
        if self.finished.is_none() {
            let _ = self.child.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end render of a 3 s synthetic clip: `cargo test -- --ignored renders_clip`
    #[test]
    #[ignore]
    fn renders_clip() {
        let dir = std::env::temp_dir().join("spectrocut-render-test");
        std::fs::create_dir_all(&dir).unwrap();
        let img = dir.join("img.jpg");
        let wav = dir.join("tone.wav");
        let ok = |args: &[&str]| {
            assert!(
                Command::new("ffmpeg")
                    .args(args)
                    .stderr(Stdio::null())
                    .status()
                    .unwrap()
                    .success()
            )
        };
        ok(&[
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=s=800x600",
            "-frames:v",
            "1",
            img.to_str().unwrap(),
        ]);
        ok(&[
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=f=440:d=3",
            wav.to_str().unwrap(),
        ]);
        let out = dir.join("out.mp4");
        let settings = Settings {
            image: img.display().to_string(),
            audio: wav.display().to_string(),
            output: out.display().to_string(),
            width: 640,
            height: 360,
            preset: "ultrafast".into(),
            ..Settings::default()
        };
        let total = crate::helpers::probe_duration(&settings.audio);
        let mut job = RenderJob::spawn(&settings, total).unwrap();
        let mut max_ratio: f64 = 0.0;
        while job.finished.is_none() {
            job.poll();
            max_ratio = max_ratio.max(job.ratio());
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        job.poll();
        println!(
            "log tail: {:?}",
            job.log.iter().rev().take(3).collect::<Vec<_>>()
        );
        assert_eq!(job.finished, Some(Ok(())));
        assert!(job.out_secs > 2.5, "out_secs = {}", job.out_secs);
        assert!(out.metadata().unwrap().len() > 1000);
        println!("max ratio seen {max_ratio}, out_secs {}", job.out_secs);
    }
}
