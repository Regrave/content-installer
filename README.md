# Content Installer

A [Calagopus Panel](https://github.com/calagopus/panel) extension for browsing, installing, and managing Minecraft plugins, mods, datapacks, and modpacks directly from the panel.

Powered by [Modrinth](https://modrinth.com) and [CurseForge](https://www.curseforge.com).

## Features

### Browse
- Search plugins, mods, and datapacks from Modrinth and CurseForge
- Card grid layout with project icons, descriptions, and download counts
- Results automatically filtered by server type (Paper, Fabric, Forge, NeoForge, etc.) and Minecraft version
- Sort by relevance, downloads, follows, newest, or recently updated
- Detail modal with full project description (rendered markdown/HTML), version selector, and one-click install
- Source toggle to switch between Modrinth and CurseForge
- Duplicate detection - warns when a mod is already installed and offers to update, removing the old version automatically

### Modpacks
- Browse and install modpacks from both Modrinth and CurseForge
- Full Modrinth `.mrpack` install support with automatic loader installation (Fabric, Forge, NeoForge, Quilt)
- Full CurseForge modpack install support with `manifest.json` parsing and per-file API resolution
- Client-only mod filtering via filename patterns, Modrinth metadata lookup, and JAR metadata inspection
- Clean install option (wipes server files before installing)
- Real-time progress tracking with file-by-file status

### Manage
- View all installed plugins/mods/datapacks with Modrinth identification via file hashing
- One-click update when newer compatible versions are available
- Remove installed content with confirmation dialog

### Datapacks
- Full datapack support for all server types (including vanilla)
- World selector when multiple worlds exist
- Reads `level-name` from `server.properties` to find the correct world directory

### CurseForge Integration
- CurseForge API key stored securely in panel settings (never exposed to the browser)
- All CurseForge API calls proxied through the panel backend
- Admin configuration page at Extensions > Content Installer to manage the API key
- Handles mods that disable third-party downloads gracefully

### Server Detection
Automatically detects your server type and Minecraft version using a deterministic decision tree:

**Server type** (checked in order, most specific first):
1. `.mcvc-type.json` marker (from [MC Version Chooser](https://github.com/Regrave/mc-version-chooser) extension)
2. `purpur.yml` > Purpur
3. `pufferfish.yml` > Pufferfish
4. `leaves.yml` > Leaves
5. `config/folia-global.yml` > Folia
6. `config/paper-global.yml` > Paper
7. `spigot.yml` > Spigot
8. `bukkit.yml` > CraftBukkit
9. `.fabric/` or `fabric-server-launch.jar` > Fabric
10. `.quilt/` or `quilt-server-launcher.jar` > Quilt
11. `libraries/net/neoforged/` > NeoForge
12. `libraries/net/minecraftforge/` > Forge
13. `server.properties` (nothing else matched) > Vanilla

**Minecraft version** (checked in order):
1. `.mcvc-type.json` marker version
2. `version.json` in server root (vanilla + Bukkit-chain)
3. Forge/NeoForge library folder name (version encoded in path)
4. `logs/latest.log` - greps for `"Starting minecraft server version"`

**Content tabs shown based on detection:**
- Vanilla > Datapacks only
- Plugin servers (Paper, Spigot, Purpur, etc.) > Plugins + Datapacks
- Mod servers (Fabric, Forge, NeoForge, Quilt) > Mods + Datapacks
- Hybrid servers (Mohist, Arclight) > Plugins + Mods + Datapacks

## Architecture

```
├── Metadata.toml              # Extension metadata
├── backend/
│   └── src/
│       ├── lib.rs             # Route registration, install/remove/modpack handlers
│       ├── curseforge.rs      # CurseForge API proxy endpoints
│       ├── settings.rs        # Extension settings (CurseForge API key)
│       └── modpack.rs         # Modpack types, progress tracking, client-only detection
├── frontend/
│   └── src/
│       ├── index.ts           # Extension entry point + route registration
│       ├── detect.ts          # Server type + MC version detection
│       ├── modrinth.ts        # Modrinth API client
│       ├── curseforge.ts      # CurseForge API client (calls backend proxy)
│       ├── AdminConfigPage.tsx # Admin settings page for CurseForge API key
│       ├── ContentInstallerPage.tsx  # Main page with tab routing
│       ├── BrowseTab.tsx      # Search + install from Modrinth/CurseForge
│       ├── ModpacksTab.tsx    # Modpack browser + installer
│       ├── ManageTab.tsx      # View + remove + update installed content
│       └── app.css            # Styling
```

## Backend API Routes

**Server routes** (`/api/client/servers/{uuid}/content-installer/`):
- `POST .../install` - Download a file to plugins/, mods/, or datapacks/
- `GET .../install/status` - Check download progress
- `POST .../remove` - Remove a file
- `POST .../modpack/install` - Install a Modrinth modpack (.mrpack)
- `POST .../modpack/cf-install` - Install a CurseForge modpack
- `GET .../modpack/status` - Check modpack install progress
- `GET .../curseforge/search` - Proxy CurseForge search
- `GET .../curseforge/files` - Proxy CurseForge file listing
- `GET .../curseforge/description` - Proxy CurseForge mod description
- `GET .../curseforge/status` - Check if CurseForge is configured

**Admin routes** (`/api/admin/content-installer/`):
- `GET .../settings` - Get extension settings (masked API key)
- `PUT .../settings` - Update extension settings

All server routes require `files.create` or `files.delete` permissions. Download URLs are validated against a whitelist of trusted CDN domains (Modrinth CDN, CurseForge CDN).

## Installation

1. Download the latest `.c7s.zip` from [Releases](https://github.com/Regrave/content-installer/releases)
2. Upload it via Admin > Extensions in your panel
3. A "Content" tab will appear in each server's sidebar
4. (Optional) Go to Admin > Extensions > Content Installer to configure CurseForge

## License

MIT
