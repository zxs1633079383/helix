# Execution Order Contract

2. `G-02` 依赖 `G-01` review-green。
3. `G-03a` 依赖 `G-02` review-green。
4. `G-04` 依赖 `G-03a` review-green。
5. `G-04a` 依赖 `G-04` review-green。
6. `G-05` 依赖 `G-04a` review-green。
7. `G-06` 依赖 `G-05` review-green。
8. `G-07` 依赖 `G-06` review-green。
9. `G-08` 依赖 `G-07` review-green。
10. `G-09` 依赖 `G-08` review-green。
11. `G-10` 依赖 `G-09` review-green。
12. `G-10a` 依赖 `G-10` review-green。
13. `G-11` 依赖 `G-10a` review-green。
14. `G-12` 依赖 `G-11` review-green。
15. `G-13` 依赖 `G-12` review-green。
16. `G-14` 依赖 `G-13` review-green。
17. `G-14a` 依赖 `G-14` review-green。
18. `G-15a` 依赖 `G-14a` review-green。
19. `G-15` 依赖 `G-15a` review-green。
20. `G-16` 依赖 `G-15` review-green。
21. `G-16b` 依赖 `G-16` review-green。
22. `G-16c` 依赖 `G-16b` review-green。
23. `G-S1` 依赖 `G-16c` review-green。
24. `G-S2` 依赖 `G-S1` review-green。
25. `G-S3` 依赖 `G-S2` review-green。
26. `G-S4` 依赖 `G-S3` review-green。

G01 为顺序 1。G03 与 G17-G27 不在 Active 分母。
