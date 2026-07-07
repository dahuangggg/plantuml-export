#!/usr/bin/env node

import fs from "node:fs";

import { renderPlantUmlTasksJson } from "../src/task-commands.mjs";

fs.writeFileSync(
  new URL("../languages/plantuml/tasks.json", import.meta.url),
  renderPlantUmlTasksJson(),
);
