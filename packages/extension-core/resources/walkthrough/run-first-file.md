# Run your first Sage file

1. Open or create a file ending in `.sage`, for example:

   ```
   R.<x> = PolynomialRing(QQ)
   f = x^3 - 2
   print(f.factor())
   ```

2. [Run Current File](command:sage.runCurrentFile), or use the ▷ button in the editor title bar.

The output appears in the **Sage** task panel. Set `sage.run.target` to `repl` to load files into a REPL session
instead, so variables stay available after the run.
