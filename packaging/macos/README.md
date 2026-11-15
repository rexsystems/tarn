# macOS DMG packaging

`dmg-background.png` is the Finder window backdrop (1320×800 @2x for a 660×400 window).

Regenerate from brand colors:

```bash
npm run dmg:bg
```

CI builds the DMG with `.github/create-macos-dmg.sh`:

- `Tarn.app` left, `Applications` right
- custom background + volume icon
- compressed UDZO
