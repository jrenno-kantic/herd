#!/usr/bin/env node

// ==============================================================================
// llama-server Models Configuration
// ==============================================================================
//
// Purpose
// -------
// Central configuration file for llama-server model presets.
// Each section defines a reusable model configuration that can be launched with:
//
//   node llama-launch.js models.ini <model-name>
//
// Examples
// --------
//   node llama-launch.js models.ini gemma-4-12b
//   node llama-launch.js models.ini qwen3.6-27b -- --ctx-size 65536
//   node llama-launch.js models.ini qwen3.6-27b -- --port 1234
//
// Configuration precedence
// ------------------------
//   1. [server]  -> common server options
//   2. [*]       -> defaults for every model
//   3. [model]   -> model-specific settings
//   4. CLI       -> command-line overrides (highest priority)
//
// Naming convention
// -----------------
// <family>-<version>-<size>-<variant>
//
// Examples:
//   gemma-4-12b
//   gemma-4-27b-qat
//   qwen3.6-27b-mtp
//   phi-4-mini
//
// Supported keys
// --------------
// model      Local GGUF path (-m)
// hf         Hugging Face repository (-hf)
// alias      llama-server alias
// mmproj     Vision projector
// ctx-size   Context length
// ngl        GPU layers
// temp       Temperature
// top-k      Top-K sampling
// top-p      Top-P sampling
// reasoning  Reasoning mode
// jinja      Enable Jinja chat template
//
// Boolean values
// --------------
// true  -> emits the flag
// false -> removes the flag
//
// Example:
//
//   jinja = true
//   flash-attn = false
//
// ==============================================================================

"use strict";

const fs = require("fs");
const path = require("path");

function usage(exitCode = 1) {
    console.error(`
Usage:
  node llama-launch.js <models.ini> <model-name> [-- extra llama-server args]

Examples:
  node llama-launch.js models.ini qwen
  node llama-launch.js models.ini gemma -- --port 1234
  node llama-launch.js models.ini qwen -- --ctx-size 65536
`);
    process.exit(exitCode);
}

const sep = process.argv.indexOf("--");

const args =
    sep === -1
        ? process.argv.slice(2)
        : process.argv.slice(2, sep);

const extraArgs =
    sep === -1
        ? []
        : process.argv.slice(sep + 1);

if (args.length !== 2)
    usage();

const iniPath = path.resolve(args[0]);
const modelName = args[1];

if (!fs.existsSync(iniPath)) {
    console.error(`Cannot find '${iniPath}'`);
    process.exit(1);
}

function parseIni(text) {
    const result = {};
    let section = null;

    for (let line of text.split(/\r?\n/)) {

        line = line.trim();

        if (!line)
            continue;

        if (line.startsWith("#") || line.startsWith(";"))
            continue;

        const s = line.match(/^\[(.+)]$/);

        if (s) {
            section = s[1].trim();
            result[section] ??= {};
            continue;
        }

        if (!section)
            continue;

        const kv = line.match(/^([^=]+?)\s*=\s*(.*?)\s*$/);

        if (!kv)
            continue;

        let key = kv[1].trim();
        let value = kv[2].trim();

        if (
            (value.startsWith('"') && value.endsWith('"')) ||
            (value.startsWith("'") && value.endsWith("'"))
        ) {
            value = value.slice(1, -1);
        }

        result[section][key] = value;
    }

    return result;
}

const config = parseIni(fs.readFileSync(iniPath, "utf8"));

if (!(modelName in config)) {

    console.error(`Unknown model '${modelName}'`);

    console.error("\nAvailable models:");

    Object.keys(config)
        .filter(x => x !== "server" && x !== "*")
        .forEach(x => console.error(`  ${x}`));

    process.exit(1);
}

function quote(v) {
    return /\s/.test(v)
        ? `"${v.replace(/"/g, '\\"')}"`
        : v;
}

/*
 * Ordered option map.
 *
 * Re-setting an existing option keeps its original position while replacing
 * its value, matching "last one wins".
 */

const options = new Map();

function setOption(flag, value = null) {
    options.set(flag, value);
}

function applySection(section) {

    if (!section)
        return;

    for (const [key, raw] of Object.entries(section)) {

        const value = raw.trim();

        let flag;

        switch (key) {

            case "model":
                flag = "-m";
                break;

            case "hf":
                flag = "-hf";
                break;

            default:
                flag = `--${key}`;
        }

        if (value.toLowerCase() === "true") {
            setOption(flag);
            continue;
        }

        if (value.toLowerCase() === "false") {
            options.delete(flag);
            continue;
        }

        setOption(flag, quote(value));
    }
}

// precedence:
//
// server
// *
// model
//

applySection(config.server);
applySection(config["*"]);
applySection(config[modelName]);

//
// CLI overrides
//

for (let i = 0; i < extraArgs.length; i++) {

    const arg = extraArgs[i];

    if (!arg.startsWith("-"))
        continue;

    const next = extraArgs[i + 1];

    if (next && !next.startsWith("-")) {
        setOption(arg, quote(next));
        i++;
    } else {
        setOption(arg);
    }
}

const cmd = ["llama-server"];

for (const [flag, value] of options) {
    cmd.push(flag);

    if (value !== null)
        cmd.push(value);
}

console.log(cmd.join(" "));