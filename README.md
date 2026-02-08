This is a simple utility to fix the OCR text found in a PDF file.

This project started as a helper utility to recover this file published by DOJ: https://www.justice.gov/epstein/files/DataSet%209/EFTA01012650.pdf

The file you found contains two photos, taken with iPhone X 4 seconds apart (2018:12:18 18:54:31.409 and the other one 4 seconds later at 18:54:35).

I was able to fix 18 pages worth of the first image. There is still minor corruption remains, but the images comes along a photo of some cloth hangers with pieces of clothing hanging on them.

I think it is unlikely to be an important photo. It is probably photos of two items of clothing someone considering buying or wearing and sent to somebody for the advice.

Here is the  result: ![decoded_resaved](./IMG_7523/decoded_resaved.jpg) (decoded with the utility and resaved to avoid being flagged for corrpted JPEG)

## About the utility

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
Ctrl+D: Display image after the processing pipeline
Ctrl+Space: cycle next characters through common OCR mistakes (O/0, g/q, etc)
Ctrl+Enter: "Finalize" the current line. Adjust the number of spaces at the end of the line to align with the PDF (AI-generated logic)
