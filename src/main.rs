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

const BASE64_ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/= ";

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

    decoded_textures: Vec<egui::TextureHandle>, // Stores the recovered JPEGs
    decode_logs: Vec<String>,                   // Stores status reports

    show_hex_dialog: bool,
    hex_input: String,
    jump_status_msg: String,
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
            decoded_textures: Vec::new(),
            decode_logs: Vec::new(),
            show_hex_dialog: false,
            hex_input: String::new(),
            jump_status_msg: String::new(),
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

                // Replace any 0x0D character with spaces
                self.text_content = self
                    .text_content
                    .chars()
                    .map(|c| if c == '\u{0D}' { ' ' } else { c })
                    .collect();

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
                    if start_byte < self.text_content.len() + 1 {
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

    fn jump_to_ilone(&self, ctx: &egui::Context) {
        let text_id = egui::Id::new("shared_pdf_editor_id");
                    
        // 1. Get current cursor position (default to 0 if not set)
        let current_idx = if let Some(state) = egui::text_edit::TextEditState::load(ctx, text_id) {
             state.cursor.char_range().map(|r| r.primary.index).unwrap_or(0)
        } else {
            0
        };

        // 2. Search for next char starting AFTER current cursor
        // We define the set of characters to look for
        let targets = ['I', 'l', '1'];
        
        // Slice the string from current_idx + 1 to end
        if current_idx + 1 < self.text_content.len() {
            let slice = &self.text_content[current_idx + 1..];
            
            // Find the offset within the slice
            if let Some(offset) = slice.find(&targets[..]) {
                let new_index = current_idx + 1 + offset;
                
                // 3. Mutate the TextEdit State
                if let Some(mut state) = egui::text_edit::TextEditState::load(ctx, text_id) {
                    // Set cursor to the new index
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(
                        egui::text::CCursor::new(new_index)
                    )));
                    
                    // Save the state back
                    state.store(ctx, text_id);
                    
                    // 4. Focus the editor so user can type immediately
                    ctx.memory_mut(|m| m.request_focus(text_id));
                }
            }
        }
    }

    // CORE LOGIC: Load files -> Clean -> Base64 -> Scan for JPEGs
    fn run_stream_decoding(&mut self, ctx: &egui::Context) {
        use base64::{Engine as _,};

        self.decoded_textures.clear();
        self.decode_logs.clear();

        // 1. Load and Sort Files
        self.decode_logs.push("Scanning current directory for page*.txt...".to_owned());
        let mut file_contents = Vec::new();
        
        if let Ok(entries) = fs::read_dir(".") {
            let mut files: Vec<_> = entries.flatten()
                .filter(|e| {
                    e.file_name().to_string_lossy().starts_with("page") 
                    && e.file_name().to_string_lossy().ends_with(".txt")
                })
                .collect();

            // Sort by number (page001, page002)
            files.sort_by_key(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                let num_str = &name[4..name.len()-4]; // strip "page" and ".txt"
                num_str.parse::<u32>().unwrap_or(9999)
            });

            for file in files {
                if let Ok(content) = fs::read_to_string(file.path()) {
                    self.decode_logs.push(format!("Loaded: {:?}", file.file_name()));
                    file_contents.push(content);
                }
            }
        }

        let raw_string = file_contents.join("");
        self.decode_logs.push(format!("Total raw length: {} characters", raw_string.len()));

        // 2. Clean Base64 Stream
        // We strip everything that isn't a Base64 data char (A-Z, a-z, 0-9, +, /).
        // We explicitly REMOVE existing '=' padding. The permissive decoder will 
        // handle the necessary padding logic internally.
        let clean_string: String = raw_string.chars()
            .filter(|c| c.is_alphanumeric() || *c == '+' || *c == '/')
            .collect();

        self.decode_logs.push(format!("Cleaned Base64 length: {} characters", clean_string.len()));

        // 3. Robust Decode
        // We configure a custom engine to be tolerant of corruption (missing padding, trailing bits).
        let config = base64::engine::GeneralPurposeConfig::new()
            .with_decode_allow_trailing_bits(true)
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent);
            
        let engine = base64::engine::GeneralPurpose::new(&base64::alphabet::STANDARD, config);

        match engine.decode(&clean_string) {
            Ok(bytes) => {
                self.decode_logs.push(format!("Decoded into {} bytes of binary data", bytes.len()));
                self.recover_jpegs_from_stream(ctx, &bytes);
            },
            Err(e) => {
                self.decode_logs.push(format!("CRITICAL: Base64 decoding failed even with permissive mode: {}", e));
            }
        }
    }

    // ROBUST SCANNER: Looks for SOI (FF D8) and handles truncated streams
    fn recover_jpegs_from_stream(&mut self, ctx: &egui::Context, bytes: &[u8]) {
        // let mut decoder = jpeg_decoder::Decoder::new(bytes);
        // let metadata = decoder.info().map(|e| self.decode_logs.push(format!("-> Got  image info: {}x{}", e.width, e.height)));
        // let pixels = decoder.decode().map_err(|e| self.decode_logs.push(format!("-> FAILED to decode image: {}", e)));

        // use zenjpeg::decoder::{Decoder, DecodedImage, DecodedImageF32, DecoderConfig};
        // if let Ok(info) = Decoder::new()
        //         .fancy_upsampling(true)
        //         .block_smoothing(false)
        //         .decode(bytes).map_err(|e| self.decode_logs.push(format!("-> FAILED to decode image: {}", e))) {
        //     // self.decode_logs.push(format!("Got image {}x{}, {} components", info.dimensions.width, info.dimensions.height, info.num_components));
        //     self.decode_logs.push(format!("Got image {}x{}", info.width, info.height));
        // }


        // let mut decoder = zune_jpeg::JpegDecoder::new(std::io::Cursor::new(bytes));
        // // decode the file
        // let pixels = decoder.decode().map_err(|e| self.decode_logs.push(format!("-> FAILED to decode image: {}", e)));


        // Attempt to decode
        match image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg) {
            Ok(img) => {
                let size = [img.width() as usize, img.height() as usize];
                let color_image = egui::ColorImage::from_rgb(size, &img.to_rgb8());
                
                let tex = ctx.load_texture(
                    "decoded_img",
                    color_image,
                    egui::TextureOptions::LINEAR
                );
                
                self.decoded_textures.push(tex);
                self.decode_logs.push("-> SUCCESS: Recovered image".into());

            },
            Err(e) => {
                self.decode_logs.push(format!("-> FAILED to decode image: {}", e));
            }
        
        }
    }


    fn perform_hex_jump(&mut self, ctx: &egui::Context) {
        // 1. Parse Hex Input
        let clean_input = self.hex_input.trim().trim_start_matches("0x");
        let binary_offset = match u64::from_str_radix(clean_input, 16) {
            Ok(val) => val,
            Err(_) => {
                self.jump_status_msg = "Invalid Hexadecimal".to_string();
                return;
            }
        };

        // 2. Calculate Target Base64 Index
        // Rule: 3 bytes of binary = 4 bytes of Base64.
        // Formula: (Offset / 3) * 4
        let target_b64_index = (binary_offset / 3) * 4;
        
        self.jump_status_msg = format!("Seeking Hex 0x{:X} -> Base64 Index {}", binary_offset, target_b64_index);

        // 3. Iterate Files
        let mut current_b64_count: u64 = 0;
        
        // Scan directory (reuse sorting logic)
        let mut files = Vec::new();
        if let Ok(entries) = fs::read_dir(".") {
            files = entries.flatten()
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    name.starts_with("page") && name.ends_with(".txt")
                })
                .collect();
            
            files.sort_by_key(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                let num_str: String = name.chars().filter(|c| c.is_ascii_digit()).collect();
                num_str.parse::<u32>().unwrap_or(9999)
            });
        }

        let mut found_page_index = None;
        let mut found_cursor_pos = 0;

        'file_loop: for file in files.iter() {
            if let Ok(content) = fs::read_to_string(file.path()) {
                // Iterate characters in this file
                for (char_idx, c) in content.chars().enumerate() {
                    // Check if it's a valid Base64 char (A-Z, a-z, 0-9, +, /)
                    // We treat everything else (newlines, spaces) as invisible to the offset count
                    if c.is_alphanumeric() || c == '+' || c == '/' {
                        if current_b64_count == target_b64_index {
                            // FOUND IT!
                            let name = file.file_name().to_string_lossy().to_string();
                            let num_str: String = name.chars().filter(|c| c.is_ascii_digit()).collect();

                            if let Ok(page_num) = num_str.parse::<u16>() {
                                // PDF pages are 0-indexed, File names are usually 1-indexed
                                found_page_index = Some(if page_num > 0 { page_num - 1 } else { 0 });
                                found_cursor_pos = char_idx;
                                break 'file_loop;
                            }
                        }
                        current_b64_count += 1;
                    }
                }
            }
        }

        // 4. Act on Result
        if let Some(idx) = found_page_index {
            // Load the page
            self.load_page(ctx, idx as u16);
            self.jump_status_msg = format!("Found on Page {}, Char {}", idx + 1, found_cursor_pos);
            self.show_hex_dialog = false; // Close dialog

            // Set Cursor and Focus
            let text_id = egui::Id::new("shared_pdf_editor_id");
            if let Some(mut state) = egui::text_edit::TextEditState::load(ctx, text_id) {
                state.cursor.set_char_range(Some(egui::text::CCursorRange::one(
                    egui::text::CCursor::new(found_cursor_pos)
                )));
                state.store(ctx, text_id);
                ctx.memory_mut(|m| m.request_focus(text_id));
            }
        } else {
            self.jump_status_msg = format!("Offset out of bounds. Max Base64 len: {}", current_b64_count);
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

                if ui.button("Jump to I/l/1").clicked() {
                    self.jump_to_ilone(ctx);
                }

                if ui.button("Hex Jump").clicked() {
                    self.show_hex_dialog = true;
                    self.hex_input.clear();
                    self.jump_status_msg.clear();
                }

                ui.separator();

                if ui.button("Save").clicked() {
                    self.save_page();
                    self.run_stream_decoding(ctx);
                }

                // Keyboard shortcuts
                if ctx.input(|i| i.key_pressed(egui::Key::S) && i.modifiers.ctrl) {
                    self.save_page();
                    self.run_stream_decoding(ctx);
                }
                if ctx.input(|i| i.key_pressed(egui::Key::J) && i.modifiers.ctrl) {
                    self.jump_to_ilone(ctx);
                }
                if ctx.input(|i| i.key_pressed(egui::Key::G) && i.modifiers.ctrl) {
                    self.show_hex_dialog = true;
                    self.hex_input.clear();
                    self.jump_status_msg.clear();
                }

                if ui.button("Display").clicked() {
                    // Determine script name based on OS
                    #[cfg(target_os = "windows")]
                    let script = "display_script.bat";
                    #[cfg(not(target_os = "windows"))]
                    let script = "./display_script.sh";

                    println!("Running script: {}", script);

                    // Execute the script
                    let child = if cfg!(target_os = "windows") {
                        Command::new("cmd").args(["/C", script]).spawn()
                    } else {
                        Command::new("sh").arg(script).spawn()
                    };
                    child.expect("Failed to launch display script");
                }
            });

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
                        let row_height = ui.fonts_mut(|f| f.row_height(&font_id)) * 1.015;

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

                            let invalid_count = line.trim().chars().filter(|&c| !BASE64_ALPHABET.contains(c)).count();
                        
                            // Check rule: Exactly 76 characters
                            let color = if invalid_count > 0 {
                                egui::Color32::ORANGE
                            } else if char_count == 76 {
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

                // let available_height = ui.available_height();

                // --- BOTTOM: DECODED IMAGES & LOGS ---
                egui::ScrollArea::vertical()
                    .id_salt("decode_scroll")
                    .show(ui, |ui| {
                        ui.heading("Decoded Stream Results");
                        
                        // 1. Show Logs
                        egui::CollapsingHeader::new("Processing Logs")
                            .default_open(true)
                            .show(ui, |ui| {
                                for log in &self.decode_logs {
                                    ui.label(log);
                                }
                            });

                        ui.separator();

                        // 2. Show Recovered Images
                        if self.decoded_textures.is_empty() {
                            ui.label("No images recovered.");
                        } else {
                            ui.label(format!("Recovered {} segments:", self.decoded_textures.len()));
                            for (i, texture) in self.decoded_textures.iter().enumerate() {
                                ui.label(format!("Segment #{}", i + 1));
                                
                                // let size = texture.size_vec2();
                                // let scale = (ui.available_width() / size.x).min(1.0); 
                                ui.image(texture);
                                ui.separator();
                            }
                        }
                    });
        });

        // --- FLOATING WINDOW FOR HEX JUMP ---
        if self.show_hex_dialog {
            egui::Window::new("Jump to Hex Offset")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.label("Enter Hex Offset (e.g., 0x2E1B):");
                    
                    // Input field
                    let response = ui.text_edit_singleline(&mut self.hex_input);
                    
                    // Auto-focus input when window opens
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.perform_hex_jump(ctx);
                    }
                    
                    // Request focus on first open
                    if self.hex_input.is_empty() && !response.has_focus() {
                        response.request_focus();
                    }

                    ui.horizontal(|ui| {
                        if ui.button("Go").clicked() {
                            self.perform_hex_jump(ctx);
                        }
                        if ui.button("Cancel").clicked() {
                            self.show_hex_dialog = false;
                        }
                    });

                    if !self.jump_status_msg.is_empty() {
                        ui.colored_label(egui::Color32::RED, &self.jump_status_msg);
                    }
                });
        }
    }
}
