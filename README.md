# epub-zipper

A small command-line tool, written in Rust, that packages a folder of loose EPUB files into a valid `.epub` file.

Run it inside an unzipped EPUB folder. It checks that the required files are present and consistent with the EPUB specification, then writes the `.epub`. If anything required is missing or invalid, it prints every problem it found and writes nothing.

> [!CAUTION]
> ## ⚠️ This is a vibe-coded project. Read this before you use it.
>
> **This code was written almost entirely by an AI through vibe coding.** It was created as a Rust learning project. It has **not** been professionally reviewed, security-audited or formally verified.
>
> - **The code may contain bugs, security vulnerabilities or logic errors**, including ones nobody has noticed yet. It could be compromised or unsafe in ways I can't rule out.
> - **The validation is not guaranteed to be correct or complete.** It tries to follow the EPUB specification, but it is *not* a replacement for [EPUBCheck](https://www.w3.org/publishing/epubcheck/), the official validator. An EPUB this tool produces may still be invalid, and one it rejects may in fact be fine.
> - **Do not use it in production, in automated pipelines, or on untrusted input** without reviewing the code yourself first.
> - **Review the source before you build or run it.** It is around 1,500 lines of Rust in `src/`. Read it, or have someone you trust read it. Do not take my word, or the AI's, that it is safe.
> If you find a bug or a security problem, please open an issue. Pull requests that improve correctness, tests or safety are welcome.

## Usage

`epub-zipper` packs **the folder you run it from** (the "current folder"). You don't tell it which folder to pack. Instead, you open a Command Prompt inside the book's folder and run the program from there.

### 1. Check your folder is an unzipped EPUB

The folder must directly contain `mimetype` and `META-INF\container.xml`, plus the book's content (the exact content folder name doesn't matter):

```
my-book\                          <-- run the tool from here
├── mimetype                      REQUIRED, must be in this exact spot
├── META-INF\                     REQUIRED, must be in this exact spot
│   └── container.xml             REQUIRED, points to your package (.opf) file
│
└── OEBPS\ (the book's content)          NAME AND LAYOUT ARE UP TO YOU
    ├── package.opf
    ├── nav.xhtml
    └── ...
```
Only `mimetype` and `META-INF\container.xml` have to be in a fixed place. Everything else can be organised however you like, because the tool finds your book by reading `container.xml`, which points to the package (`.opf`) file, and that file lists the rest. Folder names like `OEBPS` are just a common convention, not a requirement.

### 2. Open a Command Prompt in that folder

Either of these works:

- **From File Explorer:** open the book folder, click the address bar, type `cmd` and press Enter.
- **From an existing Command Prompt:** `cd /d C:\path\to\my-book`. The `/d` lets `cd` switch drives, for example from `C:` to `F:`.

### 3. Run the program

**Option A: use the full path to the `.exe`** (nothing to install):

```
C:\path\to\epub-zipper\target\release\epub-zipper.exe
```

**Option B: add it to your `PATH`** so you can just type `epub-zipper` from any folder. Copy `epub-zipper.exe` into a folder that is already on your `PATH`, or add its folder in *Settings → System → About → Advanced system settings → Environment Variables*. Then:

```
epub-zipper
```

Keep the `.exe` outside the book folder. If it is inside, it will be skipped and reported as a file that isn't part of the book.

### Choosing the output file

By default the EPUB is created in the current folder and named after that folder. Running it in `my-book\` creates `my-book\my-book.epub`. (If the folder has no name, such as a drive root like `F:\`, the file is called `book.epub`.)

To choose a different name or location, pass it as the only argument:

```
epub-zipper MyBook.epub
epub-zipper C:\Books\MyBook.epub
```

A relative name is created inside the current folder. If you give a path with folders in it, those folders must already exist. **An existing file with the same name is overwritten without asking.**

### What you'll see

**Success.** It prints where the file was created and how many files went in:

```
Created C:\path\to\my-book\my-book.epub (142 files)
```

**Warnings** are printed first and never stop the build. The most common one is for files in the folder that are not part of the book, meaning files that are not in the manifest. They are left out of the EPUB:

```
warning: 3 file(s) are not in the manifest and will NOT be packed: notes.txt, tools.txt, build.bat
```

**Errors** mean the folder is not a valid EPUB. The tool lists every problem it found, writes **no** `.epub`, and exits with a non-zero code:

```
error: `OEBPS/package.opf`: item `cover` points to `OEBPS/images/cover.jpg`, which does not exist. A file named `OEBPS/Images/cover.jpg` exists, but EPUB paths are case sensitive.
error: `OEBPS/package.opf`: spine idref `nope` is not in the manifest
error: 2 problem(s) found - no EPUB was written
```

Fix the problems, then run it again. In a script, check the result with `echo %ERRORLEVEL%`: `0` means success and `1` means it failed.

## What it does

Run in a folder laid out like an unzipped EPUB:

```
my-book\
├── mimetype
├── META-INF\
│   └── container.xml
└── OEBPS\
    ├── package.opf
    ├── nav.xhtml
    └── ...
```

it produces `my-book.epub` (named after the folder) in that same folder.

### What it checks

It works out the book's structure by reading `META-INF/container.xml` and the package document (`.opf`), so it does not depend on particular folder or file names.

- **`mimetype`:** must exist and contain exactly `application/epub+zip` (no BOM, no trailing newline).
- **`META-INF/container.xml`:** must exist, be well-formed, use the right namespace and version, and point to package documents that exist.
- **Package document:** must have the required metadata (`dc:identifier`, `dc:title`, `dc:language`), a valid `unique-identifier`, and (EPUB 3) a valid `dcterms:modified`. Both EPUB 2 and EPUB 3 are supported.
- **Manifest:** every item needs an `id`, `href` and `media-type`. Ids must be unique, and every listed file must exist. Paths are case sensitive, so `Images/` and `images/` are treated as different. Manifest paths that escape the container are rejected.
- **Spine:** must have at least one item, every `idref` must exist in the manifest, and spine items must be XHTML/SVG (or have a fallback). EPUB 2 needs its NCX table of contents reference.
- **EPUB 3 navigation document:** exactly one `nav` item containing `<nav epub:type="toc">`.
- **XML files:** XHTML, SVG, NCX and OPF files must be well-formed XML.
- **File names:** the OCF spec's forbidden characters, length limits and case-insensitive uniqueness.

### What it writes

- `mimetype` is the first entry in the zip, stored uncompressed, with no extra field. The finished zip is re-opened and checked against these rules before it is put in place.
- Everything else is Deflate-compressed.
- Only the following go into the EPUB: `mimetype`, everything in `META-INF/`, the package document, and the files listed in its manifest. Other files in the folder (notes, scripts, e.t.c.) are skipped, with a warning.
- Output is deterministic: the same folder always produces a byte-identical `.epub`.

### What it does *not* do

- It does not validate XHTML against the HTML/EPUB schemas, check that links between chapters resolve, or check images, fonts or CSS.
- It does not handle encrypted or obfuscated fonts, or multiple renditions.
- XML files encoded as UTF-16 get a warning and skip the well-formedness check.
- It is not a full replacement for EPUBCheck (see the warning above).

## Requirements

- [Rust](https://www.rust-lang.org/tools/install) **1.88 or newer**. Run `rustup update stable` if unsure.

## Build

Windows (CMD):

```
git clone https://github.com/Danny-Devito-GB/epub-zipper.git
cd epub-zipper
cargo build -r
```

The program is then at `target\release\epub-zipper.exe`. On Linux or macOS it is `target/release/epub-zipper`.

### Things to be aware of

- **An existing `.epub` with the same output name is overwritten** without asking.
- The tool only reads your source files. It writes the output `.epub` (via a temporary `.epub.tmp` file that is removed afterwards) and never modifies or deletes the files it reads. That is the intent, but see the disclaimer above.

## Project layout

```
src\
├── main.rs        command line: run the steps and print the results
├── validate.rs    decide whether the folder is valid and which files to pack
├── container.rs   checks META-INF/container.xml
├── package.rs     checks the package document (metadata, manifest, spine)
├── paths.rs       path handling and the spec's file-name rules
├── xml.rs         small helpers around the XML parser
├── report.rs      collects errors and warnings
└── pack.rs        writes the zip, then verifies it
```

Dependencies: [`zip`](https://crates.io/crates/zip) and [`roxmltree`](https://crates.io/crates/roxmltree).

## Tests

```
cargo test
```

The tests are unit tests for path handling, file-name rules and the zip header check. There are no end-to-end tests against real-world EPUBs yet, which is one more reason to treat this tool with caution.

## Specifications followed

- [EPUB 3.3](https://www.w3.org/TR/epub-33/), including the Open Container Format (OCF) rules for the zip container
- EPUB 2.0.1 (OPF 2.0) package documents
