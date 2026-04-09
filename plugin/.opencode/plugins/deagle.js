// deagle plugin for OpenCode.ai
const path = require("path");
const fs = require("fs");

module.exports = {
  name: "deagle",
  version: "0.1.0",
  init(context) {
    const skillsDir = path.join(path.dirname(__dirname), "..", "skills");
    if (context.registerSkillsPath) {
      context.registerSkillsPath("deagle", skillsDir);
    }
  },
  toolMapping: {
    Bash: "bash", Read: "read", Edit: "edit",
    Write: "write", Glob: "glob", Grep: "grep",
  },
};
