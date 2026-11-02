#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    thread,
};

use eframe::egui;
use zpl_converter::{
    Dpmm, Preset, PresetManager, config_path, convert, default_output_dir, is_zpl_path,
    read_zpl_files,
};

fn main() -> eframe::Result<()> {
    eframe::run_native(
        "ZPL Converter",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([480.0, 420.0])
                .with_min_inner_size([480.0, 420.0])
                .with_max_inner_size([480.0, 420.0])
                .with_maximized(false)
                .with_decorations(true)
                .with_always_on_top(),
            ..Default::default()
        },
        Box::new(|_| Ok(Box::new(App::new()))),
    )
}

struct App {
    files: Vec<PathBuf>,
    preset_manager: PresetManager,
    preset_load_error: Option<String>,
    selected_preset: String,
    output_dir: PathBuf,
    status: String,
    show_config: bool,
    show_edit: bool,
    edit: EditPreset,
    selected_custom: Option<String>,
    conversion: Option<Receiver<ConversionResult>>,
}

struct ConversionResult {
    output_dir: PathBuf,
    result: Result<zpl_converter::ConversionOutput, String>,
}

struct EditPreset {
    name: String,
    dpmm: Dpmm,
    width: String,
    height: String,
    original_name: Option<String>,
    error: Option<String>,
}

impl Default for EditPreset {
    fn default() -> Self {
        Self {
            name: String::new(),
            dpmm: Dpmm::Eight,
            width: String::new(),
            height: String::new(),
            original_name: None,
            error: None,
        }
    }
}

impl App {
    fn new() -> Self {
        let (presets, preset_load_error) = match PresetManager::load(&config_path()) {
            Ok(presets) => (presets, None),
            Err(error) => (PresetManager::default(), Some(error)),
        };
        let selected_preset = presets
            .names()
            .into_iter()
            .next()
            .expect("built-in presets must exist");
        let status = preset_load_error
            .clone()
            .unwrap_or_else(|| "Pronto.".to_owned());
        Self {
            files: Vec::new(),
            preset_manager: presets,
            preset_load_error,
            selected_preset,
            output_dir: default_output_dir(),
            status,
            show_config: false,
            show_edit: false,
            edit: EditPreset::default(),
            selected_custom: None,
            conversion: None,
        }
    }

    fn set_files(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() || paths.iter().any(|path| !is_zpl_path(path)) {
            self.status = "Selecione somente arquivos .zpl ou .txt.".to_owned();
            return;
        }
        self.files = paths;
        self.status = format!("{} arquivo(s) carregado(s).", self.files.len());
    }

    fn select_files(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .add_filter("Arquivos ZPL e TXT", &["zpl", "txt"])
            .pick_files()
        {
            self.set_files(paths);
        }
    }

    fn begin_conversion(&mut self) {
        let Some(preset) = self.preset_manager.find(&self.selected_preset) else {
            self.status = "Preset inválido.".to_owned();
            return;
        };
        if self.files.is_empty() {
            self.status = "Por favor, adicione pelo menos um arquivo ZPL.".to_owned();
            return;
        }
        let paths = self.files.clone();
        let output_dir = self.output_dir.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result =
                read_zpl_files(paths).and_then(|files| convert(&files, preset, &output_dir));
            let _ = sender.send(ConversionResult { output_dir, result });
        });
        self.conversion = Some(receiver);
        self.status = "Convertendo... Por favor, aguarde.".to_owned();
    }

    fn poll_conversion(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.conversion else {
            return;
        };
        match receiver.try_recv() {
            Ok(ConversionResult {
                output_dir: _,
                result: Ok(output),
            }) => {
                self.status = format!(
                    "Conversão concluída! {} PDF(s) salvo(s) em {}",
                    output.batches,
                    output.directory.display()
                );
                self.conversion = None;
            }
            Ok(ConversionResult {
                output_dir,
                result: Err(error),
            }) => {
                self.status = format!("Falha na conversão para {}: {error}", output_dir.display());
                self.conversion = None;
            }
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100))
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.status = "A conversão foi interrompida.".to_owned();
                self.conversion = None;
            }
        }
    }

    fn save_manager(&mut self, manager: PresetManager) -> Result<(), String> {
        if self.preset_load_error.is_some() {
            return Err("Corrija o arquivo de presets antes de salvar alterações.".to_owned());
        }
        manager.save(&config_path())?;
        self.preset_manager = manager;
        Ok(())
    }

    fn open_new_preset(&mut self) {
        self.edit = EditPreset::default();
        self.show_edit = true;
    }
    fn open_edit_preset(&mut self, name: String) {
        if let Some(preset) = self.preset_manager.find(&name) {
            self.edit = EditPreset {
                name: name.clone(),
                dpmm: preset.dpmm,
                width: preset.width.to_string(),
                height: preset.height.to_string(),
                original_name: Some(name),
                error: None,
            };
            self.show_edit = true;
        }
    }

    fn save_edit(&mut self) {
        let width: f32 = match self.edit.width.trim().parse::<f32>() {
            Ok(value) if value.is_finite() && value > 0.0 => value,
            _ => {
                self.edit.error = Some("Largura deve ser um número positivo e finito.".to_owned());
                return;
            }
        };
        let height: f32 = match self.edit.height.trim().parse::<f32>() {
            Ok(value) if value.is_finite() && value > 0.0 => value,
            _ => {
                self.edit.error = Some("Altura deve ser um número positivo e finito.".to_owned());
                return;
            }
        };
        let name = self.edit.name.trim().to_owned();
        let preset = Preset {
            dpmm: self.edit.dpmm,
            width,
            height,
        };
        let mut manager = self.preset_manager.clone();
        let result = if let Some(old_name) = &self.edit.original_name {
            manager.update(old_name, name.clone(), preset)
        } else {
            manager.add(name.clone(), preset)
        }
        .and_then(|()| self.save_manager(manager));
        match result {
            Ok(()) => {
                self.selected_preset = name;
                self.edit = EditPreset::default();
                self.show_edit = false;
                self.selected_custom = None;
            }
            Err(error) => self.edit.error = Some(error),
        }
    }

    fn config_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_config;
        egui::Window::new("Configurações")
            .open(&mut open)
            .resizable(true)
            .default_size([600.0, 500.0])
            .show(ctx, |ui| {
                if let Some(error) = &self.preset_load_error {
                    ui.colored_label(
                        egui::Color32::RED,
                        format!("Presets não carregados: {error}"),
                    );
                    ui.label("Corrija ou remova o arquivo de presets e reinicie o aplicativo.");
                    ui.separator();
                }
                ui.heading("Presets");
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(230.0)
                    .show(ui, |ui| {
                        for name in self.preset_manager.names() {
                            let builtin = self.preset_manager.is_builtin(&name);
                            ui.selectable_value(
                                &mut self.selected_custom,
                                Some(name.clone()),
                                if builtin {
                                    format!("{name} (padrão)")
                                } else {
                                    name
                                },
                            );
                        }
                    });
                ui.horizontal(|ui| {
                    let enabled = self.preset_load_error.is_none();
                    if ui
                        .add_enabled(enabled, egui::Button::new("Adicionar"))
                        .clicked()
                    {
                        self.open_new_preset();
                    }
                    let selected = self
                        .selected_custom
                        .clone()
                        .filter(|name| !self.preset_manager.is_builtin(name));
                    if ui
                        .add_enabled(enabled && selected.is_some(), egui::Button::new("Editar"))
                        .clicked()
                    {
                        self.open_edit_preset(selected.clone().unwrap());
                    }
                    if ui
                        .add_enabled(enabled && selected.is_some(), egui::Button::new("Remover"))
                        .clicked()
                    {
                        let name = selected.unwrap();
                        let mut manager = self.preset_manager.clone();
                        match manager
                            .remove(&name)
                            .and_then(|()| self.save_manager(manager))
                        {
                            Ok(()) => {
                                self.selected_custom = None;
                                if self.selected_preset == name {
                                    self.selected_preset = self.preset_manager.names()[0].clone();
                                }
                            }
                            Err(error) => self.status = error,
                        }
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Pasta de saída:");
                    ui.label(self.output_dir.display().to_string());
                    if ui
                        .add_enabled(self.conversion.is_none(), egui::Button::new("Alterar..."))
                        .clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .set_directory(&self.output_dir)
                            .pick_folder()
                    {
                        self.output_dir = path;
                        self.status = format!("Saída: {}", self.output_dir.display());
                    }
                });
            });
        self.show_config = open;
        if !open {
            self.show_edit = false;
            self.edit = EditPreset::default();
        }
        self.edit_window(ctx);
    }

    fn edit_window(&mut self, ctx: &egui::Context) {
        if !self.show_edit {
            return;
        }
        let title = if self.edit.original_name.is_some() {
            "Editar Preset"
        } else {
            "Adicionar Preset"
        };
        let mut open = true;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Nome:");
                ui.text_edit_singleline(&mut self.edit.name);
                ui.label("DPMM:");
                egui::ComboBox::from_id_salt("dpmm")
                    .selected_text(self.edit.dpmm.as_str())
                    .show_ui(ui, |ui| {
                        for dpmm in Dpmm::ALL {
                            ui.selectable_value(&mut self.edit.dpmm, dpmm, dpmm.as_str());
                        }
                    });
                ui.label("Largura (pol.):");
                ui.text_edit_singleline(&mut self.edit.width);
                ui.label("Altura (pol.):");
                ui.text_edit_singleline(&mut self.edit.height);
                if let Some(error) = &self.edit.error {
                    ui.colored_label(egui::Color32::RED, error);
                }
                ui.horizontal(|ui| {
                    if ui.button("Salvar").clicked() {
                        self.save_edit();
                    }
                    if ui.button("Cancelar").clicked() {
                        self.show_edit = false;
                        self.edit = EditPreset::default();
                    }
                });
            });
        if !open {
            self.show_edit = false;
            self.edit = EditPreset::default();
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if self.conversion.is_some() && ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.status = "Aguarde a conversão terminar antes de fechar o aplicativo.".to_owned();
        }
        self.poll_conversion(ctx);
        let dropped: Vec<_> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_owned())
                .collect()
        });
        if !dropped.is_empty() {
            self.set_files(dropped);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(24, 20))
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(12.0);
                    let drop_zone = egui::Frame::group(ui.style())
                        .fill(egui::Color32::from_rgb(235, 235, 245))
                        .inner_margin(30.0)
                        .show(ui, |ui| {
                            ui.set_min_size(egui::vec2(ui.available_width(), 145.0));
                            ui.vertical_centered(|ui| {
                                ui.add_space(34.0);
                                ui.heading("Arraste e solte os arquivos aqui");
                                ui.label(if self.files.is_empty() {
                                    "(ou clique para selecionar)".to_owned()
                                } else {
                                    format!("{} arquivo(s) selecionado(s)", self.files.len())
                                });
                            });
                        })
                        .response;
                    if drop_zone.interact(egui::Sense::click()).clicked() {
                        self.select_files();
                    }
                    ui.add_space(14.0);
                    egui::ComboBox::from_label("Preset")
                        .selected_text(&self.selected_preset)
                        .width(ui.available_width())
                        .show_ui(ui, |ui| {
                            for name in self.preset_manager.names() {
                                ui.selectable_value(&mut self.selected_preset, name.clone(), name);
                            }
                        });
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        let busy = self.conversion.is_some();
                        if ui
                            .add_enabled(!busy, egui::Button::new("Converter"))
                            .clicked()
                        {
                            self.begin_conversion();
                        }
                        if ui.button("Configurar").clicked() {
                            self.show_config = true;
                        }
                    });
                    ui.add_space(14.0);
                    ui.label(&self.status);
                });
            });
        if self.show_config {
            self.config_window(&ctx);
        }
    }
}
