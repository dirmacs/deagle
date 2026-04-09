# Installing deagle for Codex

## Installation

1. **Clone the repository:**
   ```bash
   git clone https://github.com/OWNER/deagle.git ~/.codex/deagle
   ```

2. **Create the skills symlink:**
   ```bash
   mkdir -p ~/.agents/skills
   ln -s ~/.codex/deagle/skills ~/.agents/skills/deagle
   ```

3. **Restart Codex** to discover the skills.

## Updating

```bash
cd ~/.codex/deagle && git pull
```
