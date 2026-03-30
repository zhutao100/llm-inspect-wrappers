> The audience is a senior SWE with sound knowledge in LLM fundamentals.

# Context

With the arise of LLM Agentic coding, more and more software development work has been shifted to the generative LLM models and the tools, like
- Codex CLI, powered by the OpenAI GPT-5.4 model as the latest
- Claude Code, powered by Anthropic Claude Opus 4.6 model as the latest

A general best-practice for project developments with the agentic tools, is empowering the LLM with toolchains to efficiently inspect the files likely relevant to the user-specified task.

## Current file inspection approaches

Currently, on BSD/GNU based OS (MacOS, Linux), the commonly seen workflow from the agentic tools is

```markdown
Understand the task form the user input and initial docs, identify
   - likely relevant keywords and symbols to look up.
   - likely relevant files to look into.

For relevant files,
- either dump to read the whole file, or dump to read the starting xxx lines first.

For relevant keywords
- leverage OS command line tools like `grep`/`rg`, `find`/`fd` to search within the project and find relevant files and line number.
- leverage `sed` to guess a rouge line number ranges and dump the ranged lines.

Based on read findings, iterate and repeat the process above, until have sufficient context for the task.
```

## Problems

The approach above often suffers from the failure modes below.

### Blindly start searching and reading into individual files, without preflight overview context

#### jumping to grep based search in big size project

E.g., given a big project of thousand files,
- the user specifies a task with prompt
- the agent does starts with keyword grep search in the root directory, without limiting the search scope
- the search returns a huge list of results, flooding the agent context window

#### repeated search and read churns for small files

E.g., given a small to medium size project of 30 code files,
- the user specifies a task with prompt
- the agent analyzes some initial context, and it points to a highly relevant code file_a
- the file_a only has 300 lines; in this case, so the most efficient context study approach would be cleanly reading the full content in one shot
- however, the agent did not inspect the file LoC, and chose a high overhead approach by
  - guess to read the 1-100 lines first, then make another tool call to read 101-200 lines; this loop has to repeat several times to finish a single file read.

This could have been avoided if the agent was given a project file layout view that
- simply show the full project layout (given there are only 30 files), along with the LoC/size info for each file

with this overview context, when the agent needs to inspect file_a, it knows both
- file_a is highly relevant
- file_a only has 300 lines

then the agent can have a very good chance of deciding to make a clean one-shot reading.

#### jumping to wrong programming language in polyglot project

E.g., given a polyglot project with core rust implementation and utility python scripts,
- the user specifies a task with prompt, and expects the function to be implemented in the core rust pipeline
- the agent starts with file search by keywords given in the user task input
- the keyword search only returns some `.py` files, making the agent think this is a Python project, then jump with python implementation for the given task


### Accidental dump of big chunks from a specific file

E.g.,
- the agent needs to inspect `server.log`
- it "smartly" uses `wc -l` to inspect the line counts, and find  the file only has 20 lines.
- then, it "confidently" dump the whole file to read.
- turns out, each line of the log is huge (e.g. long json object, or containing full dumps of big data), and the dump wastes the agent context window

In this case, this could have been avoided by a great chance if any of the conditions meet
- the agent was rendered a full project layout tree overview companied with size/LoC info
- the agent used a gated file-read executable, which gates huge lines by
  - a `[first k chars] [truncated_mark] [last k char]`
  - optionally, a heuristic line content hint, e.g. "xx size "json object with keys [...]", "xx size free text with quoted big data fields [...]"

# Task

Propose a set of **best-effort** **post-process** thin wrappers around the commands `fd` `rg` `sed`,
to help LLM agentic tools efficiently inspect projects, and avoid the failure modes stated above.

## Wrappers requirements/preferences

- the wrappers primarily targets modern MacOS; Linux compatibility is a secondary preference good to have.
- the solution is a drop-in tool to be used by the current agentic coding tools, e.g. Codex CLI.
  - the solution is preferred to be a single, low-overhead executable drop-in to run.
  - scripts that runnable with system built-in or popular frameworks (e.g. Python, Bash), if the choice brings reasonable benefits.
- the wrappers should be fast and deterministic program to run, e.g. do not nest to call LLMs to analyze the files
- the wrappers post-process the corresponding canonical tool result, and output in LLM-efficient formats
  - avoid optimizing for human readable rendering with unnecessary visual prettifiers and formats
  - do not print redundant or dumb information for the LLM, e.g. when a file name is "lib.rs", it's apparently a Rust file, do not output dumb fields like "language:Rust"
- the wrapper proxied final output should try to preserve/mimic the convention of the corresponding canonical tool, meanwhile adding low-footprint additional information

## Wrapper interface and behavior

**best-effort, passthrough fallback, never fail**

In the cases of exceptions or uncovered commands, it transparently pass through the original command output

### `fd-x`

- pass through the parameters to the canonical `fd` tool to process
- for returned the file list by `fd`, leverage `wc` to fetch the file size and LoC
- output in LLM-efficient format

Note: `fd` may need filename-stream parsing, confirm whether this is the case since it shapes the implementation choices.

### `rg-x`

similar as `fd-x`,
- pass through the parameters to the canonical `rg` tool to process
- for returned the file list by `rg`, leverage `wc` to fetch the file size and LoC
- output in LLM-efficient format

Note: `rg` should have a structured event stream, confirm whether this is the case since it shapes the implementation choices.

### `sed-x`

For now, only gate/proxy `sed -n '[start],[end]p' ...` ranged reads, pass through others.

For the ranged read,
- pass through the parameters to the canonical `rg` tool to process
- post-process the lines,
  - if encountered huge or suspicious lines, replace it with best-effort heuristic inspection result, and add a clear warning information in the final output
  - otherwise, pass through the original output
- in either case, always append the file size&LoC info at the end

### Wrapper integration recipes

For the up-to-date LLM agentic coding tools like Codex CLI, the lowest-overhead integration mechanism is adding a brief instruction sections in the master `AGENTS.md`, prompting to instruct the LLM to prefer the wrappers to inspect and analyze code files.

Note: the MCP integration option is acknowledged, however it has a repeated significantly higher overhead of making a structure MCP tool call every time.
