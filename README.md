# DownCraft toolbar skins

This branch is the skin gallery that **View → Appearance…** in DownCraft reads. Everything here
can be changed without releasing a new version of the app.

```
skin.json            list of skins (what the gallery shows)
skin/<id>.zip        the skin pack
preview/<id>.jpg     a picture of the toolbar with that skin (a wide strip is best)
```

## Add a skin

1. Put the pack in `skin/` and a preview in `preview/`, both named after the skin's `id`
   (lower-case letters, digits, `-` and `_`).
2. Add one entry to `skin.json`:

   ```json
   { "id": "my-skin", "name": "My Skin", "author": "Your name" }
   ```

   `zip` and `preview` are optional; they default to `skin/<id>.zip` and `preview/<id>.jpg`.
   Set them only when a file is named differently, e.g. `"preview": "preview/my-skin.png"`.
3. Push. The skin shows up in the gallery the next time it is opened (GitHub may cache for a few minutes).

## What a skin pack is

An IDM-style toolbar pack: a `.zip` with a `.tbi` text file and the `.bmp` icon strips it names
(12 icons side by side; a "Hot" strip is shown on hover). 32-bit BMPs use their alpha channel; 24-bit
ones use magenta (255,0,255) as the transparent colour. Packs are limited to 16 MB.
