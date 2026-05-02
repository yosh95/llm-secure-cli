# Architecture Diagram Source

This directory contains the TikZ source code for the `llm-secure-cli` architecture diagram.

## Files
- `architecture.tex`: The main TikZ source file.
- `Makefile`: Build automation for PDF and PNG generation.

## How to Compile
To generate a PDF from the `.tex` file, you can use `pdflatex`:

```bash
pdflatex architecture.tex
```

Or using `latexmk`:

```bash
latexmk -pdf architecture.tex
```

Or using the provided Makefile (generates both PDF and PNG):

```bash
make
```

## Output
The build produces:
- `architecture.pdf` — The vector PDF diagram.
- `architecture.png` — A rasterized PNG version (requires `pdftocairo`, `pdftoppm`, or ImageMagick `convert`).

The PNG output is referenced from the main project `README.md` as `images/architecture.png`.