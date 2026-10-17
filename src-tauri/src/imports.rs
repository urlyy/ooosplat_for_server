use crate::{
    error::{Result, SplatError},
    project::ImportedColmapProvenance,
    reconstruction::colmap_model,
};
use std::{collections::HashMap, path::Path};

fn validate_colmap_database(
    database: &Path,
    camera: &colmap_model::ColmapCamera,
    images: &[colmap_model::RegisteredImage],
) -> Result<()> {
    let result = (|| -> rusqlite::Result<_> {
        let connection = rusqlite::Connection::open_with_flags(
            database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let cameras = {
            let mut statement = connection
                .prepare("SELECT camera_id, model, width, height, params FROM cameras")?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, i32>(1)?,
                        row.get::<_, u64>(2)?,
                        row.get::<_, u64>(3)?,
                        row.get::<_, Vec<u8>>(4)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let database_images = {
            let mut statement = connection.prepare("SELECT name, camera_id FROM images")?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
                })?
                .collect::<rusqlite::Result<HashMap<_, _>>>()?;
            rows
        };
        Ok((cameras, database_images))
    })();
    let (cameras, database_images) = result.map_err(|error| {
        SplatError::Process(format!("无法校验导入项目的 COLMAP 数据库：{error}"))
    })?;
    let camera_matches = cameras.first().is_some_and(|stored| {
        let (chunks, remainder) = stored.4.as_chunks::<8>();
        let params = chunks
            .iter()
            .map(|bytes| f64::from_le_bytes(*bytes))
            .collect::<Vec<_>>();
        let params_match = params.len() == camera.params.len()
            && params.iter().zip(&camera.params).all(|(left, right)| {
                let tolerance = 1e-9_f64.max(right.abs() * 1e-9);
                (left - right).abs() <= tolerance
            });
        stored.0 == camera.id
            && stored.1 == camera.model_id
            && stored.2 == camera.width
            && stored.3 == camera.height
            && remainder.is_empty()
            && params_match
    });
    if cameras.len() != 1 || !camera_matches {
        return Err(SplatError::Process(
            "database.db 的相机记录与稀疏模型不一致".into(),
        ));
    }
    if images
        .iter()
        .any(|image| database_images.get(&image.name) != Some(&image.camera_id))
    {
        return Err(SplatError::Process(
            "database.db 的图像名称或相机记录与稀疏模型不一致".into(),
        ));
    }
    Ok(())
}

fn validate_registered_image_files(
    root: &Path,
    camera: &colmap_model::ColmapCamera,
    images: &[colmap_model::RegisteredImage],
) -> Result<()> {
    let canonical_root = std::fs::canonicalize(root)?;
    let expected_width = u32::try_from(camera.width)
        .map_err(|_| SplatError::Process("COLMAP 相机宽度超出图片格式范围".into()))?;
    let expected_height = u32::try_from(camera.height)
        .map_err(|_| SplatError::Process("COLMAP 相机高度超出图片格式范围".into()))?;
    for image in images {
        let path = canonical_root.join(&image.name);
        let path = std::fs::canonicalize(&path).map_err(|_| {
            SplatError::Process(format!("原始图片目录缺少已注册图像：{}", image.name))
        })?;
        if !path.starts_with(&canonical_root) || !path.is_file() {
            return Err(SplatError::Process(format!(
                "已注册图像路径无效：{}",
                image.name
            )));
        }
        let dimensions = image::ImageReader::open(&path)
            .and_then(|reader| reader.with_guessed_format())
            .map_err(|error| {
                SplatError::Process(format!("无法读取已注册图像 {}：{error}", image.name))
            })?
            .into_dimensions()
            .map_err(|error| {
                SplatError::Process(format!("无法读取已注册图像 {}：{error}", image.name))
            })?;
        if dimensions != (expected_width, expected_height) {
            return Err(SplatError::Process(format!(
                "已注册图像 {} 的尺寸与相机记录不一致",
                image.name
            )));
        }
    }
    Ok(())
}

pub(crate) fn revalidate_imported_colmap(
    colmap: &ImportedColmapProvenance,
) -> Result<(
    colmap_model::ColmapCamera,
    Vec<colmap_model::RegisteredImage>,
)> {
    let database = colmap
        .database_path
        .as_ref()
        .filter(|path| path.is_file())
        .ok_or_else(|| SplatError::Process("外部 COLMAP 项目缺少 database.db".into()))?;
    let images_root = colmap
        .images_path
        .as_ref()
        .filter(|path| path.is_dir())
        .ok_or_else(|| SplatError::Process("外部 COLMAP 项目缺少原始图片".into()))?;
    if !colmap.sparse_model_path.is_dir() {
        return Err(SplatError::Process("外部 COLMAP 项目缺少稀疏模型".into()));
    }
    let camera = colmap_model::read_single_camera(&colmap.sparse_model_path)?;
    let images = colmap_model::read_registered_images(&colmap.sparse_model_path)?;
    if images.is_empty() {
        return Err(SplatError::Process("稀疏模型没有已注册图像".into()));
    }
    if images.iter().any(|image| image.camera_id != camera.id) {
        return Err(SplatError::Process(
            "外部项目包含多个相机，不能进行同相机增量补拍".into(),
        ));
    }
    validate_colmap_database(database, &camera, &images)?;
    validate_registered_image_files(images_root, &camera, &images)?;
    Ok((camera, images))
}

#[cfg(test)]
mod tests {
    use super::validate_colmap_database;
    use tempfile::tempdir;

    #[test]
    fn imported_database_camera_rejects_malformed_parameter_blobs() {
        let root = tempdir().unwrap();
        let database = root.path().join("database.db");
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE cameras (camera_id INTEGER PRIMARY KEY, model INTEGER, width INTEGER, height INTEGER, params BLOB);
                 CREATE TABLE images (name TEXT, camera_id INTEGER);",
            )
            .unwrap();
        let camera = crate::reconstruction::colmap_model::ColmapCamera {
            id: 1,
            model_id: 2,
            model: "SIMPLE_RADIAL".into(),
            width: 640,
            height: 480,
            params: vec![600.5, 320.25, 240.75, 0.125],
        };
        let valid: Vec<u8> = camera
            .params
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        connection
            .execute("INSERT INTO cameras VALUES (1, 2, 640, 480, ?1)", [&valid])
            .unwrap();
        validate_colmap_database(&database, &camera, &[]).unwrap();

        let mut trailing_byte = valid.clone();
        trailing_byte.push(0);
        for malformed in [
            trailing_byte,
            valid[..valid.len() - 1].to_vec(),
            valid[..valid.len() - 8].to_vec(),
            vec![0_u8; valid.len()],
            Vec::new(),
        ] {
            connection
                .execute("UPDATE cameras SET params = ?1", [&malformed])
                .unwrap();
            assert!(validate_colmap_database(&database, &camera, &[]).is_err());
        }
    }
}
