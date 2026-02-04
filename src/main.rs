use eframe::egui;
use pdfium_render::prelude::*;
use std::env;
use std::sync::Arc;

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
        viewport: egui::ViewportBuilder::default().with_inner_size([800.0, 900.0]),
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
            app.load_page(&cc.egui_ctx, 0);
        } else {
            app.text_content = format!("Could not load PDF at path: {}", path);
        }

        app
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
            });

            let available_height = ui.available_height();

            // --- TOP SECTION: PDF VIEW ---
            egui::ScrollArea::vertical()
                .max_height(available_height * 0.5)
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

                        let text_id = ui.make_persistent_id("text_editor");
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

                                    let screen_rect =
                                        egui::Rect::from_min_max(screen_min, screen_max);

                                    // FIX #1: Green Stroke instead of Fill
                                    painter.rect_stroke(
                                        screen_rect,
                                        0.0,
                                        egui::Stroke::new(2.0, egui::Color32::GREEN),
                                        egui::StrokeKind::Outside,
                                    );
                                }
                            }
                        }
                    }
                });

            ui.separator();

            // --- BOTTOM SECTION: TEXT EDITOR ---
            egui::ScrollArea::vertical()
                .id_salt("text_scroll")
                .show(ui, |ui| {
                    let text_edit = egui::TextEdit::multiline(&mut self.text_content)
                        .id(ui.make_persistent_id("text_editor"))
                        .desired_width(f32::INFINITY)
                        .font(egui::FontId::new(16.0, egui::FontFamily::Monospace));

                    ui.add(text_edit);
                });
        });
    }
}
