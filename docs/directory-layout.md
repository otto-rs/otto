~/.otto/
  <name>-<path-hash>/                   # e.g. otto-a1b2c3d4/
    .cache/                             # Long-term script storage
      task1/
        e3b0c4...                      # Hash of task1's script content
        f4d5e6...                      # Different version of task1's script
      task2/
        a1b2c3...                      # Hash of task2's script content
        b2c3d4...                      # Different version of task2's script
    <timestamp>/                        # e.g. 1710424593/
      tasks/
        task1/
          script.sh -> ../../../.cache/task1/e3b0c4...  # Symlink to cached script
          stdout.log
          stderr.log
          output.json                   # Task outputs for downstream tasks
          artifacts/                    # Task-generated files
        task2/
          script.py -> ../../../.cache/task2/a1b2c3...  # Symlink to cached script
          stdout.log
          stderr.log
          input-task1.json -> ../task1/output.json  # Named symlinks to upstream outputs
          output.json
          artifacts/
      run.yaml                         # Copy of otto.yml used for this run
      env.yaml                         # Environment state when run started
      metadata.yaml                    # Run metadata (start time, duration, etc)
      cmdline.yaml                     # Original command line args
