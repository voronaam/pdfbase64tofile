use eframe::egui;
use eframe::emath;
use pdfium_render::prelude::*;
use std::env;
use std::fs;
use std::process::Command;

fn main() -> Result<(), eframe::Error> {
    // 1. Setup PDFium
    // Ensure the dynamic library (dll/dylib/so) is available at runtime
    let pdfium = Pdfium::new(
        Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./"))
            .or_else(|_| Pdfium::bind_to_system_library())
            .expect(
                "Could not load PDFium library. Please ensure the dynamic library is available.",
            ),
    );

    // Sadly, this thing loads a C++ library and has to live forever
    let pdfium_static: &'static Pdfium = Box::leak(Box::new(pdfium));

    // 2. Load File from CLI
    let args: Vec<String> = env::args().collect();
    let file_path = if args.len() > 1 {
        args[1].clone()
    } else {
        eprintln!("Usage: cargo run -- <path_to_pdf>");
        "test.pdf".to_string()
    };

    // 3. Initialize App State
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_maximized(true),
        ..Default::default()
    };

    eframe::run_native(
        "PDF text to JPEG utility",
        options,
        Box::new(|cc| Ok(Box::new(PdfApp::new(cc, pdfium_static, file_path)))),
    )
}

struct PdfApp {
    // PDF State
    document: Option<PdfDocument<'static>>,
    current_page_index: u16,
    total_pages: u16,

    // Visual State
    page_texture: Option<egui::TextureHandle>,
    page_size: egui::Vec2,

    // Text State
    text_content: String,

    _pdfium: &'static Pdfium,
}

impl PdfApp {
    fn new(cc: &eframe::CreationContext<'_>, pdfium: &'static Pdfium, path: String) -> Self {
        let mut app = Self {
            document: None,
            current_page_index: 0,
            total_pages: 0,
            page_texture: None,
            page_size: egui::Vec2::ZERO,
            text_content: String::new(),
            _pdfium: pdfium,
        };

        if let Ok(doc) = pdfium.load_pdf_from_file(&path, None) {
            app.total_pages = doc.pages().len();
            app.document = Some(doc);
            app.load_page(&cc.egui_ctx, Self::latest_index());
        } else {
            app.text_content = format!("Could not load PDF at path: {}", path);
        }

        app
    }

    fn latest_index() -> u16 {
        let mut max_index = 0;
    
        if let Ok(entries) = fs::read_dir(".") {
            for entry in entries.flatten() {
                if let Some(file_name) = entry.file_name().to_str() {
                    if let Some(captures) = file_name.strip_prefix("page").and_then(|s| s.strip_suffix(".txt")) {
                        if let Ok(index) = captures.parse::<u16>() {
                            if index > max_index {
                                max_index = index - 1;
                            }
                        }
                    }
                }
            }
        }
    
        max_index
    }

    fn load_page(&mut self, ctx: &egui::Context, index: u16) {
        if let Some(doc) = &self.document {
            if let Ok(page) = doc.pages().get(index) {
                // 1. Render Page to Bitmap
                let bitmap = page.render(2000, 2000, None).unwrap();
                let image = bitmap.as_image();
                let size = [image.width() as usize, image.height() as usize];
                let pixels = image.into_rgb8();

                // 2. Upload to Egui GPU Texture
                let color_image = egui::ColorImage::from_rgb(size, &pixels);
                self.page_texture =
                    Some(ctx.load_texture("pdf_page", color_image, egui::TextureOptions::LINEAR));

                self.page_size = egui::vec2(page.width().value, page.height().value);

                // 3. Extract Text
                if let Ok(text) = page.text() {
                    self.text_content = text.all();
                }

                // 4. If the file exists, load its text
                let file_name = format!("page{:03}.txt", index + 1);
                if let Ok(content) = std::fs::read_to_string(&file_name) {
                    eprintln!("Loading file {}", file_name);
                    self.text_content = content;
                }

                self.current_page_index = index;
            }
        }
    }

    fn get_highlights(&self, selection: egui::text::CCursorRange) -> Vec<egui::Rect> {
        let mut rects = Vec::new();

        if let Some(doc) = &self.document {
            if let Ok(page) = doc.pages().get(self.current_page_index) {
                let boundaries = page.boundaries();
                let crop = boundaries
                    .crop()
                    .unwrap_or(boundaries.media().expect("Neither crop no media present"));

                let p_width = crop.bounds.width().value;
                let p_height = crop.bounds.height().value;
                let p_left_offset = crop.bounds.left().value;
                let _p_bottom_offset = crop.bounds.bottom().value;
                // In PDF, 'top' is the highest Y value.
                // We use this to flip the Y-axis.
                let p_top_value = crop.bounds.top().value;

                if let Ok(text_page) = page.text() {
                    // Egui gives us Byte Indices
                    let start_byte = selection.primary.index.min(selection.secondary.index);
                    let end_byte = selection.primary.index.max(selection.secondary.index);

                    // Conversion: Byte Index -> Char Index
                    if start_byte < self.text_content.len() {
                        let start_char_idx = self.text_content[..start_byte].chars().count();

                        let char_count = if start_byte == end_byte {
                            1
                        } else {
                            self.text_content[start_byte..end_byte].chars().count()
                        };

                        for char_obj in text_page
                            .chars()
                            .iter()
                            .skip(start_char_idx)
                            .take(char_count)
                        {
                            if let Ok(rect) = char_obj.loose_bounds() {
                                // We calculate coordinates RELATIVE to the page dimensions (0.0 to 1.0)
                                // This helps if the rendered image has been cropped or scaled differently.
                                let left_pct = (rect.left().value - p_left_offset) / p_width;
                                let top_pct = (p_top_value - rect.top().value) / p_height;
                                let width_pct = (rect.right().value - rect.left().value) / p_width;
                                let height_pct =
                                    (rect.top().value - rect.bottom().value) / p_height;

                                rects.push(egui::Rect::from_min_size(
                                    egui::pos2(left_pct, top_pct),
                                    egui::vec2(width_pct, height_pct),
                                ));
                            }
                        }
                    }
                }
            }
        }
        rects
    }

    fn save_page(&self) {
        let filename = format!("page{:03}.txt", self.current_page_index + 1);

        if let Err(e) = fs::write(&filename, &self.text_content) {
            eprintln!("Error saving file {}: {}", filename, e);
        } else {
            println!("Saved text to {}", filename);
        }
    }
}

impl eframe::App for PdfApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Prev").clicked() && self.current_page_index > 0 {
                    self.load_page(ctx, self.current_page_index - 1);
                }
                ui.label(format!(
                    "Page {} / {}",
                    self.current_page_index + 1,
                    self.total_pages
                ));
                if ui.button("Next").clicked() && self.current_page_index < self.total_pages - 1 {
                    self.load_page(ctx, self.current_page_index + 1);
                }

                ui.separator();

                if ui.button("Save").clicked() {
                    self.save_page();
                }
                if ctx.input(|i| i.key_pressed(egui::Key::S) && i.modifiers.ctrl) {
                    self.save_page();
                }

                if ui.button("Display").clicked() {
                    // Determine script name based on OS
                    #[cfg(target_os = "windows")]
                    let script = "display_script.bat";
                    #[cfg(not(target_os = "windows"))]
                    let script = "./display_script.sh";

                    println!("Running script: {}", script);

                    // Execute the script
                    let output = if cfg!(target_os = "windows") {
                        Command::new("cmd").args(["/C", script]).output()
                    } else {
                        Command::new("sh").arg(script).output()
                    };

                    match output {
                        Ok(o) => {
                            if o.status.success() {
                                println!("Script output: {}", String::from_utf8_lossy(&o.stdout));
                            } else {
                                eprintln!("Script failed: {}", String::from_utf8_lossy(&o.stderr));
                            }
                        }
                        Err(e) => eprintln!("Failed to execute script: {}", e),
                    }
                }
            });

            let available_height = ui.available_height();

            // --- TOP SECTION: PDF VIEW ---
            egui::ScrollArea::vertical()
                .max_height(80.0)// .max_height(available_height * 0.1)
                .id_salt("pdf_scroll")
                .show(ui, |ui| {
                    if let Some(texture) = &self.page_texture {
                        let size = texture.size_vec2();
                        let scale = ui.available_width() / size.x;
                        let display_size = size * scale;

                        let (rect, _response) =
                            ui.allocate_exact_size(display_size, egui::Sense::click());
                        let painter = ui.painter_at(rect);
                        painter.image(
                            texture.id(),
                            rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );

                        let text_id = egui::Id::new("shared_pdf_editor_id");
                        if let Some(state) = egui::text_edit::TextEditState::load(ctx, text_id) {
                            if let Some(range) = state.cursor.char_range() {
                                let highlights = self.get_highlights(range);

                                if let Some(first_rect) = highlights.first() {
                                    let screen_min = rect.min
                                        + egui::vec2(
                                            first_rect.min.x * display_size.x,
                                            first_rect.min.y * display_size.y,
                                        );
                                    let screen_max = rect.min
                                        + egui::vec2(
                                            first_rect.max.x * display_size.x,
                                            first_rect.max.y * display_size.y,
                                        );
                                    let cursor_screen_rect =
                                        egui::Rect::from_min_max(screen_min, screen_max);

                                    // Tell Egui to scroll here if it's off-screen
                                    // None = Minimal scroll (just bring it into view)
                                    // Some(Align::Center) = Always center it
                                    ui.scroll_to_rect(cursor_screen_rect, None);
                                }

                                for h_rect_norm in highlights {
                                    // Convert normalized coordinates (0..1) back to Screen Pixels
                                    let screen_min = rect.min
                                        + egui::vec2(
                                            h_rect_norm.min.x * display_size.x,
                                            h_rect_norm.min.y * display_size.y,
                                        );
                                    let screen_max = rect.min
                                        + egui::vec2(
                                            h_rect_norm.max.x * display_size.x,
                                            h_rect_norm.max.y * display_size.y,
                                        );

                                    // Rectangle mode
                                    // let screen_rect =
                                    //     egui::Rect::from_min_max(screen_min, screen_max);

                                    // painter.rect_stroke(
                                    //     screen_rect,
                                    //     0.0,
                                    //     egui::Stroke::new(2.0, egui::Color32::GREEN),
                                    //     egui::StrokeKind::Outside,
                                    // );

                                    // Underline mode
                                    let line_y = screen_max.y; // Bottom of the rectangle
                                    let line_start = egui::pos2(screen_min.x - 2.0, line_y); // Extend slightly to the left
                                    let line_end = egui::pos2(screen_max.x + 2.0, line_y);   // Extend slightly to the right
                                
                                    // Draw a bold green line under the letter
                                    painter.line_segment(
                                        [line_start, line_end],
                                        egui::Stroke::new(4.0, egui::Color32::GREEN), // Bold line
                                    );

                                }
                            }
                        }
                    }
                });

            ui.separator();

            // --- BOTTOM SECTION: SPLIT EDITOR ---
            egui::ScrollArea::vertical()
                .id_salt("text_scroll")
                .max_height(80.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // SETUP FONTS
                        let font_size = 24.0;
                        // We define the font here so we can use metrics for both the indicator and the editor
                        let font_id = egui::FontId::new(font_size, egui::FontFamily::Monospace);
                        let row_height = ui.fonts_mut(|f| f.row_height(&font_id)) * 1.02;

                        // 1. LEFT PANEL: STATUS INDICATORS
                        // We allocate a vertical strip. Width = 15px.
                        // Height = total lines * row height.
                        let total_lines = self.text_content.lines().count().max(1);
                        let desired_height = total_lines as f32 * row_height;

                        // Allocate space for the indicators
                        let (rect, _response) = ui.allocate_exact_size(
                            egui::vec2(15.0, desired_height),
                            egui::Sense::hover(),
                        );

                        // Draw the indicators
                        let painter = ui.painter_at(rect);
                        for (i, line) in self.text_content.lines().enumerate() {
                            let char_count = line.trim().chars().count();

                            // Check rule: Exactly 76 characters
                            let color = if char_count == 76 {
                                egui::Color32::GREEN
                            } else {
                                egui::Color32::from_gray(50) // Dim gray for other lines
                            };

                            // Calculate position
                            // Note: TextEdit usually adds a small margin (approx 4.0-8.0px).
                            // We offset Y slightly to align with the text baseline.
                            let y_offset = rect.top() + (i as f32 * row_height) + 4.0;

                            painter.rect_filled(
                                egui::Rect::from_min_size(
                                    egui::pos2(rect.left(), y_offset),
                                    egui::vec2(8.0, row_height - 2.0),
                                ),
                                2.0, // rounding
                                color,
                            );
                        }

                        let text_id = egui::Id::new("shared_pdf_editor_id");
                        let text_edit = egui::TextEdit::multiline(&mut self.text_content)
                            .id(text_id)
                            .desired_width(f32::INFINITY)
                            .horizontal_align(emath::Align::Center)
                            .font(egui::FontId::new(font_size, egui::FontFamily::Monospace));

                        ui.add(text_edit);
                    });
                });
        });
    }
}
