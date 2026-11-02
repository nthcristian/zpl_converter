use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

pub const MAX_PRESET_NAME_LEN: usize = 35;
const MAX_LABEL_DIMENSION_INCHES: f32 = 15.0;
const MAX_LABELS_PER_REQUEST: usize = 50;
const MAX_REQUEST_BYTES: usize = 1_000_000;
const MAX_LABELS_PER_CONVERSION: usize = 10_000;
pub const BUILTIN_PRESETS: [(&str, Preset); 2] = [
    (
        "Etiqueta 10x15cm (8 dpmm)",
        Preset {
            dpmm: Dpmm::Eight,
            width: 3.90,
            height: 5.90,
        },
    ),
    (
        "Etiqueta 40x25mm 2col (8 dpmm)",
        Preset {
            dpmm: Dpmm::Eight,
            width: 314.0 / 100.0,
            height: 1.00,
        },
    ),
];

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub enum Dpmm {
    #[serde(rename = "6dpmm")]
    Six,
    #[serde(rename = "8dpmm")]
    Eight,
    #[serde(rename = "12dpmm")]
    Twelve,
    #[serde(rename = "24dpmm")]
    TwentyFour,
}

impl Dpmm {
    pub const ALL: [Self; 4] = [Self::Six, Self::Eight, Self::Twelve, Self::TwentyFour];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Six => "6dpmm",
            Self::Eight => "8dpmm",
            Self::Twelve => "12dpmm",
            Self::TwentyFour => "24dpmm",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Preset {
    pub dpmm: Dpmm,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SavedPreset {
    pub name: String,
    #[serde(flatten)]
    pub preset: Preset,
}

#[derive(Clone, Default)]
pub struct PresetManager {
    custom: Vec<SavedPreset>,
}

impl PresetManager {
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let data =
            fs::read_to_string(path).map_err(|error| format!("Falha ao ler presets: {error}"))?;
        let custom: Vec<SavedPreset> = serde_json::from_str(&data)
            .map_err(|error| format!("Falha ao decodificar presets: {error}"))?;
        let manager = Self { custom };
        manager.validate_all()?;
        Ok(manager)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        self.validate_all()?;
        let parent = path.parent().ok_or("Caminho de configuração inválido")?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("Falha ao criar diretório de configuração: {error}"))?;
        let mut custom = self.custom.clone();
        custom.sort_by(|left, right| left.name.cmp(&right.name));
        let data = serde_json::to_vec_pretty(&custom)
            .map_err(|error| format!("Falha ao serializar presets: {error}"))?;
        atomic_write(path, &data).map_err(|error| format!("Falha ao salvar presets: {error}"))
    }

    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<_> = BUILTIN_PRESETS
            .iter()
            .map(|(name, _)| (*name).to_owned())
            .chain(self.custom.iter().map(|preset| preset.name.clone()))
            .collect();
        names.sort();
        names
    }

    pub fn find(&self, name: &str) -> Option<Preset> {
        BUILTIN_PRESETS
            .iter()
            .find_map(|(builtin_name, preset)| (*builtin_name == name).then_some(*preset))
            .or_else(|| {
                self.custom
                    .iter()
                    .find(|saved| saved.name == name)
                    .map(|saved| saved.preset)
            })
    }

    pub fn is_builtin(&self, name: &str) -> bool {
        BUILTIN_PRESETS
            .iter()
            .any(|(builtin_name, _)| *builtin_name == name)
    }

    pub fn add(&mut self, name: String, preset: Preset) -> Result<(), String> {
        self.validate_new_name(&name, None)?;
        validate_preset(preset)?;
        self.custom.push(SavedPreset { name, preset });
        Ok(())
    }

    pub fn update(&mut self, old_name: &str, name: String, preset: Preset) -> Result<(), String> {
        if self.is_builtin(old_name) {
            return Err(format!(
                "Não é possível editar o preset padrão '{old_name}'"
            ));
        }
        self.validate_new_name(&name, Some(old_name))?;
        validate_preset(preset)?;
        let saved = self
            .custom
            .iter_mut()
            .find(|saved| saved.name == old_name)
            .ok_or("Preset não encontrado")?;
        saved.name = name;
        saved.preset = preset;
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> Result<(), String> {
        if self.is_builtin(name) {
            return Err(format!("Não é possível remover o preset padrão '{name}'"));
        }
        let initial_len = self.custom.len();
        self.custom.retain(|saved| saved.name != name);
        if self.custom.len() == initial_len {
            Err("Preset não encontrado".to_owned())
        } else {
            Ok(())
        }
    }

    fn validate_all(&self) -> Result<(), String> {
        let mut names = HashSet::new();
        for saved in &self.custom {
            if self.is_builtin(&saved.name) {
                return Err(format!(
                    "O preset '{}' usa o nome de um preset padrão",
                    saved.name
                ));
            }
            if saved.name.trim().is_empty() {
                return Err("O nome não pode estar vazio".to_owned());
            }
            if saved.name.chars().count() > MAX_PRESET_NAME_LEN {
                return Err(format!("O nome excede {MAX_PRESET_NAME_LEN} caracteres"));
            }
            if !names.insert(&saved.name) {
                return Err(format!("Há mais de um preset com o nome '{}'", saved.name));
            }
            validate_preset(saved.preset)?;
        }
        Ok(())
    }

    fn validate_new_name(&self, name: &str, existing_name: Option<&str>) -> Result<(), String> {
        if name.trim().is_empty() {
            return Err("O nome não pode estar vazio".to_owned());
        }
        if name.chars().count() > MAX_PRESET_NAME_LEN {
            return Err(format!("O nome excede {MAX_PRESET_NAME_LEN} caracteres"));
        }
        if self
            .names()
            .iter()
            .any(|candidate| candidate == name && Some(candidate.as_str()) != existing_name)
        {
            return Err(format!("Já existe um preset com o nome '{name}'"));
        }
        Ok(())
    }
}

fn validate_preset(preset: Preset) -> Result<(), String> {
    if !preset.width.is_finite()
        || !preset.height.is_finite()
        || preset.width <= 0.0
        || preset.height <= 0.0
    {
        return Err("Largura e altura devem ser números positivos e finitos".to_owned());
    }
    if preset.width > MAX_LABEL_DIMENSION_INCHES || preset.height > MAX_LABEL_DIMENSION_INCHES {
        return Err(format!(
            "Largura e altura não podem exceder {MAX_LABEL_DIMENSION_INCHES} polegadas"
        ));
    }
    Ok(())
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("zpl-converter")
        .join("presets.json")
}
pub fn default_output_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn read_zpl_files(paths: impl IntoIterator<Item = PathBuf>) -> Result<Vec<Vec<u8>>, String> {
    let paths: Vec<_> = paths.into_iter().collect();
    if paths.is_empty() {
        return Err("Nenhum arquivo foi selecionado.".to_owned());
    }
    if paths.iter().any(|path| !is_zpl_path(path)) {
        return Err("Selecione somente arquivos .zpl ou .txt.".to_owned());
    }
    paths
        .into_iter()
        .map(|path| {
            fs::read(&path).map_err(|error| format!("Falha ao ler {}: {error}", path.display()))
        })
        .collect()
}

pub fn is_zpl_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("zpl") || extension.eq_ignore_ascii_case("txt")
        })
}

pub fn split_into_batches(files: &[Vec<u8>], batch_size: usize) -> Result<Vec<Vec<u8>>, String> {
    if batch_size == 0 {
        return Err("O tamanho do lote deve ser maior que zero.".to_owned());
    }
    let mut batches = Vec::new();
    let mut current = Vec::new();
    let mut count = 0;
    let mut total: usize = 0;
    for file in files {
        let formats = parse_formats(file)?;
        for format in formats {
            let mut quantity = format.quantity;
            total = total
                .checked_add(quantity)
                .ok_or("Quantidade total de etiquetas inválida.")?;
            if total > MAX_LABELS_PER_CONVERSION {
                return Err(format!(
                    "A conversão excede o limite de {MAX_LABELS_PER_CONVERSION} etiquetas."
                ));
            }
            while quantity > 0 {
                if count == batch_size {
                    batches.push(std::mem::take(&mut current));
                    count = 0;
                }
                let label_count = quantity.min(batch_size - count);
                let label = format.with_quantity(label_count);
                if label.len() + 1 > MAX_REQUEST_BYTES {
                    return Err("Uma etiqueta excede o limite de 1 MB da API Labelary.".to_owned());
                }
                if !current.is_empty() && current.len() + label.len() + 1 > MAX_REQUEST_BYTES {
                    batches.push(std::mem::take(&mut current));
                    count = 0;
                    continue;
                }
                current.extend_from_slice(&label);
                current.push(b'\n');
                count += label_count;
                quantity -= label_count;
            }
        }
    }
    if !current.is_empty() {
        batches.push(current);
    }
    if batches.is_empty() {
        return Err("Não foi encontrada nenhuma etiqueta ZPL (^XA ... ^XZ).".to_owned());
    }
    Ok(batches)
}

struct Format<'a> {
    bytes: &'a [u8],
    quantity: usize,
    quantity_digits: Option<(usize, usize)>,
}

struct ParsedFormat {
    end: usize,
    prefix: u8,
    quantity: usize,
    quantity_digits: Option<(usize, usize)>,
}
impl Format<'_> {
    fn with_quantity(&self, quantity: usize) -> Vec<u8> {
        let Some((start, end)) = self.quantity_digits else {
            return self.bytes.to_vec();
        };
        let mut result = Vec::with_capacity(self.bytes.len() + 8);
        result.extend_from_slice(&self.bytes[..start]);
        result.extend_from_slice(quantity.to_string().as_bytes());
        result.extend_from_slice(&self.bytes[end..]);
        result
    }
}

fn parse_formats(file: &[u8]) -> Result<Vec<Format<'_>>, String> {
    let mut formats = Vec::new();
    let mut cursor = 0;
    let mut prefix = b'^';
    while let Some(start) = find_command(&file[cursor..], prefix, b"XA") {
        let start = cursor + start;
        let parsed = parse_format(&file[start..], prefix)?;
        formats.push(Format {
            bytes: &file[start..start + parsed.end],
            quantity: parsed.quantity,
            quantity_digits: parsed.quantity_digits,
        });
        cursor = start + parsed.end;
        prefix = parsed.prefix;
    }
    Ok(formats)
}

fn parse_format(label: &[u8], mut prefix: u8) -> Result<ParsedFormat, String> {
    let mut index = 3;
    let mut quantity = 1;
    let mut quantity_digits = None;
    while index + 2 < label.len() {
        if label[index] != prefix {
            index += 1;
            continue;
        }
        let command = &label[index + 1..index + 3];
        if command.eq_ignore_ascii_case(b"XZ") {
            return Ok(ParsedFormat {
                end: index + 3,
                prefix,
                quantity,
                quantity_digits,
            });
        }
        if command.eq_ignore_ascii_case(b"CC") {
            let Some(next_prefix) = label.get(index + 3) else {
                return Err("Comando ^CC sem novo prefixo.".to_owned());
            };
            prefix = *next_prefix;
            index += 4;
            continue;
        }
        if command.eq_ignore_ascii_case(b"PQ") {
            let start = index + 3;
            let end = label[start..]
                .iter()
                .position(|byte| !byte.is_ascii_digit())
                .map_or(label.len(), |offset| start + offset);
            if end > start {
                quantity = std::str::from_utf8(&label[start..end])
                    .ok()
                    .and_then(|value| value.parse().ok())
                    .filter(|quantity: &usize| (1..=99_999_999).contains(quantity))
                    .ok_or("Quantidade ^PQ inválida (use 1 a 99999999).")?;
                quantity_digits = Some((start, end));
            } else {
                quantity = 1;
                quantity_digits = Some((start, start));
            }
            index = end;
            continue;
        }
        index += 3;
    }
    Err("Etiqueta ZPL sem terminador XZ.".to_owned())
}

fn find_command(haystack: &[u8], prefix: u8, command: &[u8; 2]) -> Option<usize> {
    haystack
        .windows(3)
        .position(|window| window[0] == prefix && window[1..].eq_ignore_ascii_case(command))
}

pub struct ConversionOutput {
    pub batches: usize,
    pub directory: PathBuf,
}

pub fn convert(
    files: &[Vec<u8>],
    preset: Preset,
    output_dir: &Path,
) -> Result<ConversionOutput, String> {
    validate_preset(preset)?;
    let batches = split_into_batches(files, MAX_LABELS_PER_REQUEST)?;
    fs::create_dir_all(output_dir)
        .map_err(|error| format!("Falha ao criar diretório de saída: {error}"))?;
    let destination = unique_output_directory(output_dir)?;
    let temporary = output_dir.join(format!(
        ".{}-tmp",
        destination
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
    ));
    fs::create_dir(&temporary).map_err(|error| format!("Falha ao preparar saída: {error}"))?;
    let url = format!(
        "https://api.labelary.com/v1/printers/{}/labels/{}x{}",
        preset.dpmm.as_str(),
        preset.width,
        preset.height
    );
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("Falha ao criar cliente HTTP: {error}"))?;
    let result = (|| {
        for (index, batch) in batches.iter().enumerate() {
            if index > 0 {
                thread::sleep(Duration::from_secs(3));
            }
            let response = client
                .post(&url)
                .header("Accept", "application/pdf")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(batch.clone())
                .send()
                .map_err(|error| format!("Falha na requisição: {error}"))?;
            let status = response.status();
            let bytes = response
                .bytes()
                .map_err(|error| format!("Falha ao ler resposta: {error}"))?;
            if !status.is_success() {
                return Err(format!(
                    "Erro da API Labelary ({status}): {}",
                    String::from_utf8_lossy(&bytes)
                ));
            }
            atomic_write(
                &temporary.join(format!("labels_batch_{}.pdf", index + 1)),
                &bytes,
            )
            .map_err(|error| format!("Falha ao salvar PDF: {error}"))?;
        }
        fs::rename(&temporary, &destination)
            .map_err(|error| format!("Falha ao finalizar saída: {error}"))?;
        Ok(ConversionOutput {
            batches: batches.len(),
            directory: destination,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&temporary);
    }
    result
}

fn unique_output_directory(output_dir: &Path) -> Result<PathBuf, String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("Relógio do sistema inválido: {error}"))?
        .as_millis();
    let stem = format!("zpl-conversion-{timestamp}-{}", std::process::id());
    for suffix in 0..1_000 {
        let name = if suffix == 0 {
            stem.clone()
        } else {
            format!("{stem}-{suffix}")
        };
        let path = output_dir.join(name);
        if !path.exists() {
            return Ok(path);
        }
    }
    Err("Não foi possível criar um diretório de saída único.".to_owned())
}

fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, data)?;
    fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_labels_across_files_into_requested_batch_size() {
        let files = vec![b"skip^XAone^XZ^XAtwo^XZ".to_vec(), b"^XAthree^XZ".to_vec()];
        assert_eq!(
            split_into_batches(&files, 2).unwrap(),
            vec![
                b"^XAone^XZ\n^XAtwo^XZ\n".to_vec(),
                b"^XAthree^XZ\n".to_vec()
            ]
        );
    }

    #[test]
    fn splits_print_quantity_across_batches() {
        let label = b"^XA^FO20,20^FDRepeated label^FS^PQ60^XZ".to_vec();
        assert_eq!(
            split_into_batches(&[label], 50).unwrap(),
            vec![
                b"^XA^FO20,20^FDRepeated label^FS^PQ50^XZ\n".to_vec(),
                b"^XA^FO20,20^FDRepeated label^FS^PQ10^XZ\n".to_vec()
            ]
        );
    }

    #[test]
    fn honors_changed_format_prefix_for_end_and_quantity_commands() {
        let label = b"^XA^CC!!FO20,20!FDRepeated label!FS!PQ60!XZ".to_vec();
        assert_eq!(
            split_into_batches(&[label], 50).unwrap(),
            vec![
                b"^XA^CC!!FO20,20!FDRepeated label!FS!PQ50!XZ\n".to_vec(),
                b"^XA^CC!!FO20,20!FDRepeated label!FS!PQ10!XZ\n".to_vec()
            ]
        );
    }

    #[test]
    fn rejects_unterminated_or_excessive_formats() {
        assert!(split_into_batches(&[b"^XAunfinished".to_vec()], 50).is_err());
        assert!(split_into_batches(&[b"^XA^PQ10001^XZ".to_vec()], 50).is_err());
    }

    #[test]
    fn rejects_invalid_preset() {
        let mut presets = PresetManager::default();
        assert!(
            presets
                .add(
                    "Invalid".to_owned(),
                    Preset {
                        dpmm: Dpmm::Eight,
                        width: f32::INFINITY,
                        height: 1.0
                    }
                )
                .is_err()
        );
    }

    #[test]
    fn rejects_builtin_presets_loaded_from_disk() {
        let path = std::env::temp_dir().join(format!("zpl-presets-{}.json", std::process::id()));
        fs::write(
            &path,
            r#"[{"name":"Etiqueta 10x15cm (8 dpmm)","dpmm":"8dpmm","width":1,"height":1}]"#,
        )
        .unwrap();
        assert!(PresetManager::load(&path).is_err());
        let _ = fs::remove_file(path);
    }
}
