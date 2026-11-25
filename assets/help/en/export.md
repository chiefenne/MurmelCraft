+++
covers = ["window.export_image", "window.export_movie", "window.screenshot", "window.copy_image", "export.*", "view.screenshot"]
related = ["series", "views"]
chapter = "export"
+++
Turn the view into a picture or a movie. **Export image…** writes a PNG at any size, with an optional legend, axis triad and transparent background. **Copy the view as an image** puts the view on the clipboard as it is shown.

<!-- more -->

## Pictures and movies

- **Export image…**: size in pixels, legend and axes each on or off, a transparent background. Lines and text scale with the picture, so a large export does not look thin.
- **Export movie…**: an orbit, a flight between saved views, or the steps of a time series. GIF is built in. MP4 needs `ffmpeg` on your system.
- **Save a screenshot…** writes the view as it is, at the window's size.
- **Copy the view as an image** renders the view at the viewport's size in pixels and puts it on the clipboard, ready to paste into a report.

The time label of a series step can go into pictures and movies.
