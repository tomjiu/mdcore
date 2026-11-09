#!/usr/bin/env bash
# #598 探针：子目录工作区下 status 有变化但 diff 空（先红证据）
set -u
R=/tmp/mdc-598-probe/repo
rm -rf "$R"; mkdir -p "$R/nested/deep"
cd "$R" || exit 1
git init -q && git config user.email p@local && git config user.name p
echo "基线" > nested/tracked.txt
echo "deep 基线" > nested/deep/deep.txt
git add -A && git commit -qm init
echo "基线" > /dev/null
printf '基线\n修改行\n' > nested/tracked.txt
echo "--- A) 仓库根: status + diff ---"
git status --porcelain=v1 -z --untracked-files=all | tr '\0' '\n'
git diff HEAD -- nested/tracked.txt | wc -c
echo "--- B) 一级子目录 root=nested: status + diff（现状缺陷面）---"
git -C nested status --porcelain=v1 -z --untracked-files=all | tr '\0' '\n'
echo "git -C nested diff HEAD -- nested/tracked.txt 字节数:"
git -C nested diff HEAD -- nested/tracked.txt | wc -c
echo "--- C) 仓库根口径（正确答案）---"
echo "git diff HEAD -- nested/tracked.txt 字节数:"
git diff HEAD -- nested/tracked.txt | wc -c
echo "--- D) rev-parse 基准定位 ---"
git -C nested rev-parse --show-toplevel
git -C nested rev-parse --show-prefix
git -C nested/deep rev-parse --show-toplevel
git -C nested/deep rev-parse --show-prefix
echo "--- E) 相似名不串读: 顶层 a.txt 未改, nested/a.txt 修改 ---"
echo 顶 > a.txt; git add a.txt; git commit -qm top
echo 巢 > nested/a.txt
echo "root=nested status:"
git -C nested status --porcelain=v1 -z --untracked-files=all | tr '\0' '\n'
echo "root=nested diff HEAD -- nested/a.txt 字节数:"
git -C nested diff HEAD -- nested/a.txt | wc -c
echo "root=nested diff HEAD -- a.txt 字节数（若误按仓库根解释会串读顶层 a.txt）:"
git -C nested diff HEAD -- a.txt | wc -c
