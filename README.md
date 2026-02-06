This is a simple utility to fix the OCR text found in a PDF file.

The expectation is that PDF file contains image and text, text in
the PDF is base64 encoded image formatted to have 76 characters per line.

It stores the decoded files in current directory. As in `page001.txt`, `page002.txt` and so on.

Depends on pdfium library from Google.

Download from pdfium for your platform and place in the current folder. E.g. `libpdfium.so` for Linux.

```
cargo run -- ~/Downloads/EFTA01012650.pdf
```

Feel free to edit `display_script.sh` to ise whatever extra post processing you want to add.

My script just uses `tr`, `base64` and `display`.

Screenshot: https://imgur.com/screenshot-gTnNrkW

Shortcuts:

Ctrl+S: Save
Ctrl+J: Jump to next I/l/1
Ctrl+G: Jump to hex address
Ctrl+Space: cycle next characters through common OCR mistakes (O/0, g/q, etc)
