use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use zip::write::FileOptions;

use crate::repro::scrub::ScrubConfig;
use crate::repro::tape::{ReproTape, REPRO_TAPE_SCHEMA_VERSION};

pub const REPRO_BUNDLE_SCHEMA_VERSION: u32 = 1;

const META_JSON: &str = "meta.json";
const TAPE_JSONL: &str = "tape.jsonl";
const DB_SQLITE: &str = "db.sqlite";
const WORKSPACE_PATCH: &str = "workspace.patch";

pub const CONDUIT_DB_FILENAME: &str = "conduit.db";
pub const REPRO_DIRNAME: &str = "repro";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReproExportMode {
    /// Optimized for local debugging: minimal scrubbing, may include more context.
    Local,
    /// Optimized for sharing: aggressive scrubbing and conservative contents.
    Shareable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReproBundleMeta {
    pub schema_version: u32,
    pub export_mode: ReproExportMode,
    pub created_at_ms: u64,
    pub app_version: String,
    pub os: String,
    pub git_commit: Option<String>,
}

pub struct ReproBundleOpen {
    pub meta: ReproBundleMeta,
    pub temp_dir: TempDir,
    pub db_path: PathBuf,
    pub tape_path: PathBuf,
    pub workspace_patch_path: Option<PathBuf>,
}

pub struct ReproPreparedDataDir {
    pub meta: ReproBundleMeta,
    /// The temporary directory holding the prepared data dir contents.
    ///
    /// Keep this alive for as long as you want the `data_dir` contents to exist.
    pub temp_dir: TempDir,
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub tape_path: PathBuf,
    pub workspace_patch_path: Option<PathBuf>,
}

pub struct ReproBundle;

impl ReproBundle {
    pub fn create(
        out_path: &Path,
        mut meta: ReproBundleMeta,
        mut tape: ReproTape,
        db_path: &Path,
        workspace_patch: Option<&str>,
    ) -> anyhow::Result<()> {
        // Keep the two schema versions in sync for now (bundle can evolve independently later).
        meta.schema_version = REPRO_BUNDLE_SCHEMA_VERSION;
        if tape.schema_version == 0 {
            tape.schema_version = REPRO_TAPE_SCHEMA_VERSION;
        }

        if meta.export_mode == ReproExportMode::Shareable {
            let scrub = ScrubConfig::default_shareable();
            scrub.scrub_tape(&mut tape);
        }

        let zip_file = File::create(out_path)?;
        let mut zip = zip::ZipWriter::new(zip_file);
        let options = FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        // meta.json
        zip.start_file(META_JSON, options)?;
        let meta_json = serde_json::to_vec_pretty(&meta)?;
        zip.write_all(&meta_json)?;

        // tape.jsonl
        zip.start_file(TAPE_JSONL, options)?;
        let mut tape_buf = Vec::new();
        tape.write_jsonl_to(&mut tape_buf)?;
        zip.write_all(&tape_buf)?;

        // db.sqlite
        zip.start_file(DB_SQLITE, options)?;
        let mut db_file = File::open(db_path)?;
        io::copy(&mut db_file, &mut zip)?;

        // workspace.patch (optional)
        if let Some(patch) = workspace_patch {
            zip.start_file(WORKSPACE_PATCH, options)?;
            zip.write_all(patch.as_bytes())?;
        }

        let mut zip_file = zip.finish()?;
        zip_file.flush()?;
        Ok(())
    }

    pub fn open(path: &Path) -> anyhow::Result<ReproBundleOpen> {
        let file = File::open(path)?;
        let mut archive = zip::ZipArchive::new(file)?;

        let mut meta: ReproBundleMeta = {
            let mut entry = archive.by_name(META_JSON)?;
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            serde_json::from_slice(&buf)?
        };

        if meta.schema_version > REPRO_BUNDLE_SCHEMA_VERSION {
            anyhow::bail!(
                "unsupported repro bundle schema_version {} (max supported: {})",
                meta.schema_version,
                REPRO_BUNDLE_SCHEMA_VERSION
            );
        }

        let temp_dir = tempfile::tempdir()?;
        let db_path = temp_dir.path().join(DB_SQLITE);
        let tape_path = temp_dir.path().join(TAPE_JSONL);
        let workspace_patch_path = temp_dir.path().join(WORKSPACE_PATCH);

        // Extract db and tape into temp dir.
        {
            let mut src = archive.by_name(DB_SQLITE)?;
            let mut dst = File::create(&db_path)?;
            io::copy(&mut src, &mut dst)?;
        }
        {
            let mut src = archive.by_name(TAPE_JSONL)?;
            let mut dst = File::create(&tape_path)?;
            io::copy(&mut src, &mut dst)?;
        }

        let workspace_patch_path = match archive.by_name(WORKSPACE_PATCH) {
            Ok(mut src) => {
                let mut dst = File::create(&workspace_patch_path)?;
                io::copy(&mut src, &mut dst)?;
                Some(workspace_patch_path)
            }
            Err(_) => None,
        };

        // Best-effort: ensure meta is sane even if older bundles omitted fields.
        if meta.schema_version == 0 {
            meta.schema_version = REPRO_BUNDLE_SCHEMA_VERSION;
        }

        Ok(ReproBundleOpen {
            meta,
            temp_dir,
            db_path,
            tape_path,
            workspace_patch_path,
        })
    }

    /// Prepare a Conduit-compatible `data_dir` layout for a repro bundle.
    ///
    /// This creates a temp directory containing:
    /// - `conduit.db` (the database snapshot)
    /// - `repro/meta.json` / `repro/tape.jsonl` / optional `repro/workspace.patch`
    pub fn prepare_data_dir(bundle_path: &Path) -> anyhow::Result<ReproPreparedDataDir> {
        let opened = Self::open(bundle_path)?;
        let data_dir = opened.temp_dir.path().to_path_buf();

        let db_out = data_dir.join(CONDUIT_DB_FILENAME);
        std::fs::copy(&opened.db_path, &db_out)?;

        let repro_dir = data_dir.join(REPRO_DIRNAME);
        std::fs::create_dir_all(&repro_dir)?;

        let meta_out = repro_dir.join(META_JSON);
        std::fs::write(meta_out, serde_json::to_vec_pretty(&opened.meta)?)?;

        let tape_out = repro_dir.join(TAPE_JSONL);
        std::fs::copy(&opened.tape_path, &tape_out)?;

        let workspace_patch_path = match opened.workspace_patch_path {
            Some(path) => {
                let out = repro_dir.join(WORKSPACE_PATCH);
                std::fs::copy(path, &out)?;
                Some(out)
            }
            None => None,
        };

        Ok(ReproPreparedDataDir {
            meta: opened.meta,
            temp_dir: opened.temp_dir,
            data_dir,
            db_path: db_out,
            tape_path: tape_out,
            workspace_patch_path,
        })
    }

    /// Extract a repro bundle into a Conduit `data_dir` folder on disk.
    pub fn extract_to_data_dir(
        bundle_path: &Path,
        out_dir: &Path,
        overwrite: bool,
    ) -> anyhow::Result<()> {
        if out_dir.exists() && !out_dir.is_dir() {
            anyhow::bail!("output path is not a directory: {}", out_dir.display());
        }
        std::fs::create_dir_all(out_dir)?;

        let db_out = out_dir.join(CONDUIT_DB_FILENAME);
        if db_out.exists() && !overwrite {
            anyhow::bail!(
                "refusing to overwrite existing {} (pass --overwrite to replace)",
                db_out.display()
            );
        }

        let repro_dir = out_dir.join(REPRO_DIRNAME);
        if repro_dir.exists() && !overwrite {
            anyhow::bail!(
                "refusing to overwrite existing {} (pass --overwrite to replace)",
                repro_dir.display()
            );
        }
        std::fs::create_dir_all(&repro_dir)?;

        let opened = Self::open(bundle_path)?;

        std::fs::copy(&opened.db_path, &db_out)?;
        std::fs::write(
            repro_dir.join(META_JSON),
            serde_json::to_vec_pretty(&opened.meta)?,
        )?;
        std::fs::copy(&opened.tape_path, repro_dir.join(TAPE_JSONL))?;
        if let Some(patch) = opened.workspace_patch_path {
            std::fs::copy(patch, repro_dir.join(WORKSPACE_PATCH))?;
        }

        Ok(())
    }
}

trait ReproTapeWriteExt {
    fn write_jsonl_to<W: Write>(&self, w: &mut W) -> io::Result<()>;
}

impl ReproTapeWriteExt for ReproTape {
    fn write_jsonl_to<W: Write>(&self, w: &mut W) -> io::Result<()> {
        // Avoid duplicating the file-only helper, but keep bundle creation streaming-friendly.
        // Serialize header + entries in the same format used by `ReproTape::write_jsonl_to_path`.
        use crate::repro::tape::REPRO_TAPE_SCHEMA_VERSION;
        use serde_json::json;

        let header = json!({
            "type": "header",
            "schema_version": self.schema_version.max(REPRO_TAPE_SCHEMA_VERSION),
            "created_at_ms": self.created_at_ms,
        });
        writeln!(
            w,
            "{}",
            serde_json::to_string(&header).map_err(io::Error::other)?
        )?;
        for entry in &self.entries {
            let line = json!({
                "type": "entry",
                "entry": entry,
            });
            // This relies on ReproTapeEntry being serializable, which it is.
            writeln!(
                w,
                "{}",
                serde_json::to_string(&line).map_err(io::Error::other)?
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repro::tape::ReproTapeEntry;
    use tempfile::tempdir;

    #[test]
    fn bundle_roundtrip_contains_expected_files() {
        let dir = tempdir().unwrap();
        let out = dir.path().join("bundle.repro.zip");
        let db = dir.path().join("db.sqlite");
        std::fs::write(&db, b"not a real db").unwrap();

        let meta = ReproBundleMeta {
            schema_version: REPRO_BUNDLE_SCHEMA_VERSION,
            export_mode: ReproExportMode::Local,
            created_at_ms: 1,
            app_version: "0.0.0-test".to_string(),
            os: "test".to_string(),
            git_commit: None,
        };
        let mut tape = ReproTape::new();
        tape.entries.push(ReproTapeEntry::Note {
            seq: 1,
            ts_ms: 1,
            message: "hello".to_string(),
        });

        ReproBundle::create(&out, meta, tape, &db, Some("diff")).unwrap();
        let opened = ReproBundle::open(&out).unwrap();

        assert!(opened.db_path.exists());
        assert!(opened.tape_path.exists());
        assert!(opened.workspace_patch_path.is_some());
    }

    #[test]
    fn prepare_data_dir_creates_conduit_layout() {
        let dir = tempdir().unwrap();
        let bundle = dir.path().join("bundle.repro.zip");
        let db = dir.path().join("db.sqlite");
        std::fs::write(&db, b"not a real db").unwrap();

        let meta = ReproBundleMeta {
            schema_version: REPRO_BUNDLE_SCHEMA_VERSION,
            export_mode: ReproExportMode::Local,
            created_at_ms: 1,
            app_version: "0.0.0-test".to_string(),
            os: "test".to_string(),
            git_commit: None,
        };
        let tape = ReproTape::new();

        ReproBundle::create(&bundle, meta, tape, &db, Some("diff")).unwrap();
        let prepared = ReproBundle::prepare_data_dir(&bundle).unwrap();

        assert!(prepared.db_path.exists());
        assert!(prepared.tape_path.exists());
        assert!(prepared.data_dir.join(REPRO_DIRNAME).is_dir());
        assert!(prepared
            .data_dir
            .join(REPRO_DIRNAME)
            .join(META_JSON)
            .exists());
        assert!(prepared.workspace_patch_path.is_some());
    }

    #[test]
    fn extract_to_data_dir_writes_conduit_layout() {
        let dir = tempdir().unwrap();
        let bundle = dir.path().join("bundle.repro.zip");
        let db = dir.path().join("db.sqlite");
        std::fs::write(&db, b"not a real db").unwrap();

        let meta = ReproBundleMeta {
            schema_version: REPRO_BUNDLE_SCHEMA_VERSION,
            export_mode: ReproExportMode::Local,
            created_at_ms: 1,
            app_version: "0.0.0-test".to_string(),
            os: "test".to_string(),
            git_commit: None,
        };
        let tape = ReproTape::new();

        ReproBundle::create(&bundle, meta, tape, &db, Some("diff")).unwrap();

        let out_dir = dir.path().join("out_data_dir");
        ReproBundle::extract_to_data_dir(&bundle, &out_dir, false).unwrap();

        assert!(out_dir.join(CONDUIT_DB_FILENAME).exists());
        assert!(out_dir.join(REPRO_DIRNAME).join(META_JSON).exists());
        assert!(out_dir.join(REPRO_DIRNAME).join(TAPE_JSONL).exists());
        assert!(out_dir.join(REPRO_DIRNAME).join(WORKSPACE_PATCH).exists());
    }
}
