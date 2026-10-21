use super::{
    model::{Session, State},
    validation::{valid_id, validate},
};
use crate::adapters::project::Project;
use harness_domain::{Failure, execution::OperationContext};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    id: String,
    data: State,
}

fn safe_metadata(path: &Path) -> Result<fs::Metadata, Failure> {
    let meta =
        fs::symlink_metadata(path).map_err(|_| ("PERSISTENCE", "ruta de sesión no accesible"))?;
    if meta.file_type().is_symlink() {
        return Err(("PERSISTENCE", "enlace en ruta de sesión"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(("PERSISTENCE", "reparse point en ruta de sesión"));
        }
    }
    Ok(meta)
}

pub(super) fn directory(project: &Project, create: bool) -> Result<Option<PathBuf>, Failure> {
    let mut path = project.root.clone();
    for part in [".agents", "harness", ".state"] {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if !create {
                    return Ok(None);
                }
                let builder = fs::DirBuilder::new();
                #[cfg(unix)]
                let builder = {
                    use std::os::unix::fs::DirBuilderExt;
                    let mut builder = builder;
                    builder.mode(0o700);
                    builder
                };
                match builder.create(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(_) => {
                        return Err(("PERSISTENCE", "no se pudo crear directorio de sesiones"));
                    }
                }
            }
            Err(_) => {
                return Err((
                    "PERSISTENCE",
                    "no se pudo inspeccionar directorio de sesiones",
                ));
            }
            Ok(_) => {}
        }
        if !safe_metadata(&path)?.is_dir()
            || path
                .canonicalize()
                .map_err(|_| ("PERSISTENCE", "directorio no accesible"))?
                != path
        {
            return Err(("PERSISTENCE", "directorio de sesión no seguro"));
        }
    }
    Ok(Some(path))
}

pub(super) fn save(project: &Project, id: &str, state: &State) -> Result<(), Failure> {
    if !valid_id(id) {
        return Err(("SESSION", "identificador de sesión inválido"));
    }
    let dir = directory(project, true)?.expect("created directory");
    let file = dir.join(format!("{id}.json"));
    match fs::symlink_metadata(&file) {
        Ok(_) => {
            if !safe_metadata(&file)?.is_file() {
                return Err(("PERSISTENCE", "archivo de sesión no seguro"));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let files =
                fs::read_dir(&dir).map_err(|_| ("PERSISTENCE", "no se pudo listar sesiones"))?;
            let mut count = 0;
            for entry in files {
                let entry = entry.map_err(|_| ("PERSISTENCE", "no se pudo listar sesiones"))?;
                if entry.path().extension().is_some_and(|ext| ext == "json") {
                    count += 1;
                }
            }
            if count >= 256 {
                return Err(("PERSISTENCE", "límite de 256 sesiones persistentes"));
            }
        }
        Err(_) => return Err(("PERSISTENCE", "archivo de sesión no accesible")),
    }
    let bytes = serde_json::to_vec(&Saved {
        version: 1,
        id: id.into(),
        data: state.clone(),
    })
    .map_err(|_| ("PERSISTENCE", "sesión no serializable"))?;
    if bytes.len() > 256 * 1024 {
        return Err(("PERSISTENCE", "sesión demasiado grande"));
    }
    let mut temporary = tempfile::NamedTempFile::new_in(&dir)
        .map_err(|_| ("PERSISTENCE", "no se pudo crear sesión temporal"))?;
    temporary
        .write_all(&bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|_| ("PERSISTENCE", "no se pudo escribir sesión"))?;
    temporary
        .persist(&file)
        .map_err(|_| ("PERSISTENCE", "no se pudo publicar sesión"))?;
    Ok(())
}

pub(super) fn load(project: &Project, id: &str) -> Result<Session, Failure> {
    if !valid_id(id) {
        return Err(("SESSION", "identificador de sesión inválido"));
    }
    let dir = directory(project, false)?.ok_or((
        "SESSION",
        "sesión inexistente o descartada; ejecutar harness_bootstrap",
    ))?;
    let file = dir.join(format!("{id}.json"));
    let meta = safe_metadata(&file).map_err(|_| ("SESSION", "sesión no accesible o no segura"))?;
    if !meta.is_file() || meta.len() > 256 * 1024 {
        return Err(("SESSION", "archivo de sesión no admitido"));
    }
    let bytes = project.bytes(&format!(".agents/harness/.state/{id}.json"))?;
    if bytes.len() > 256 * 1024 {
        return Err(("SESSION", "archivo de sesión no admitido"));
    }
    let saved: Saved =
        serde_json::from_slice(&bytes).map_err(|_| ("SESSION", "estado persistente inválido"))?;
    if saved.version != 1 || saved.id != id {
        return Err(("SESSION", "formato de sesión incompatible"));
    }
    let mut state = saved.data;
    validate(&mut state)?;
    Ok(Session {
        tasks: harness_domain::workflow::TaskBoard::restore(state.phases.clone(), &super::now()),
        state,
        restored: true,
        ..Session::default()
    })
}

pub(in crate::adapters) fn persist_snapshot(
    project: &Project,
    id: &str,
    snapshot: &Value,
    context: &OperationContext,
) -> Result<(), Failure> {
    context.check()?;
    let mut state: State = serde_json::from_value(snapshot.clone())
        .map_err(|_| ("PERSISTENCE", "Invalid session snapshot"))?;
    validate(&mut state)?;
    save(project, id, &state)
}
