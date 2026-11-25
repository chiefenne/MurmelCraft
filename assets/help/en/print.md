+++
covers = ["window.export_pdf", "window.print", "window.page_setup", "export.pdf", "settings.set_page"]
related = ["export", "settings"]
chapter = "export"
+++
**Print…** puts the view on a page and opens your system's print dialog. **Export PDF page…** writes the same page as a PDF file. **Page setup…** chooses the paper, the orientation, the margins and the resolution.

<!-- more -->

## Print and PDF

The view fills the page inside its margins, with its colour legend and axes. Above it go a title (the mesh's file name) and a line naming MurmelCraft, the date and the file; turn the header off in Page setup.

- **Paper**: A4 (the default), A3 or Letter, landscape (the default) or portrait.
- **Margins**: 12 mm on every side unless you change them.
- **Resolution**: 200 dpi by default. A large page at a high resolution is drawn at less, so that the picture stays within what the renderer can make.
- **Theme**: light by default, because dark backgrounds print badly; choose dark to print what the window shows.

Page setup is kept in `[page]` of settings.toml, so every page you print or export uses it. On macOS, Print opens the page in Preview's print dialog; elsewhere it opens in your PDF viewer, where you print it. Agents write the page with `export.pdf`.
