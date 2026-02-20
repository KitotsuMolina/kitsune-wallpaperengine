# Release + AUR quick guide

## 1) Crear release en GitHub

1. Sube tus cambios a `main`.
2. Crea y sube un tag:

```bash
git tag v0.2.0
git push origin v0.2.0
```

3. El workflow `.github/workflows/release.yml` compila y publica artefactos en GitHub Releases.

## 2) Publicar en AUR

1. Clona tu repo AUR (créalo primero en aurweb):

```bash
git clone ssh://aur@aur.archlinux.org/kitsune-livewallpaper.git aur-kitsune-livewallpaper
```

2. Copia `aur/PKGBUILD` y `aur/.SRCINFO`:

```bash
cp aur/PKGBUILD aur/.SRCINFO aur-kitsune-livewallpaper/
```

3. Ajusta versión si hace falta:
- `pkgver`
- URL del `source` con `v${pkgver}`

4. Genera `.SRCINFO` actualizado:

```bash
cd aur-kitsune-livewallpaper
makepkg --printsrcinfo > .SRCINFO
```

5. Commit y push a AUR:

```bash
git add PKGBUILD .SRCINFO
git commit -m "release: v0.2.0"
git push
```
