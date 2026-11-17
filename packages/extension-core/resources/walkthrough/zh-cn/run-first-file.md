# 运行第一个 Sage 文件

1. 打开或新建一个以 `.sage` 结尾的文件，例如：

   ```
   R.<x> = PolynomialRing(QQ)
   f = x^3 - 2
   print(f.factor())
   ```

2. [运行当前文件](command:sage.runCurrentFile)，或点击编辑器标题栏上的 ▷ 按钮。

输出显示在 **Sage** 任务面板中。把 `sage.run.target` 设为 `repl` 后，文件会加载到 REPL 会话里运行，运行结束后变量仍然可用。
