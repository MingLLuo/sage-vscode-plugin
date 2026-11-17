# Install or select SageMath

Editing features (highlighting, hover, go to definition, completion, references) work without a Sage installation.
Running files, the REPL, and runtime documentation need SageMath.

- **Sage is installed:** the extension finds `sage` on your `PATH` and common install locations automatically. The
  status bar shows the detected version, for example `Sage: ready (SageMath 10.8)`.
- **Sage is somewhere else or not detected:** [Select Sage Interpreter](command:sage.selectInterpreter) and pick a
  detected installation or enter the path to the `sage` executable.
- **Sage is not installed:** follow the [SageMath installation guide](https://doc.sagemath.org/html/en/installation/index.html).
  On Windows, install Sage inside WSL and open the folder with the WSL extension.

If the status bar says `Sage not found`, the configured path cannot be started.
