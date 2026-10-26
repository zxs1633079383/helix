#!/usr/bin/env python3
"""md -> 自包含 HTML（marked.js + mermaid.js + highlight.js, 带 TOC）。
用法: python3 md2html.py <input.md> <output.html> [标题]
"""
import sys, base64, pathlib

def main():
    if len(sys.argv) < 3:
        print("usage: md2html.py <in.md> <out.html> [title]"); sys.exit(1)
    src, dst = sys.argv[1], sys.argv[2]
    title = sys.argv[3] if len(sys.argv) > 3 else pathlib.Path(src).stem
    md = pathlib.Path(src).read_text(encoding="utf-8")
    b64 = base64.b64encode(md.encode("utf-8")).decode("ascii")

    html = """<!DOCTYPE html>
<html lang="zh">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>__TITLE__</title>
<script src="https://cdn.jsdelivr.net/npm/marked/marked.min.js"></script>
<script src="https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.min.js"></script>
<link rel="stylesheet" href="https://cdn.jsdelivr.net/gh/highlightjs/cdn-release/build/styles/github-dark.min.css">
<script src="https://cdn.jsdelivr.net/gh/highlightjs/cdn-release/build/highlight.min.js"></script>
<style>
:root{--bg:#0d1117;--panel:#161b22;--border:#30363d;--fg:#e6edf3;--muted:#8b949e;--accent:#58a6ff;--accent2:#3fb950;}
*{box-sizing:border-box;}
body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.7 -apple-system,BlinkMacSystemFont,"Segoe UI","PingFang SC","Microsoft YaHei",sans-serif;}
#layout{display:flex;min-height:100vh;}
#toc{width:300px;flex-shrink:0;border-right:1px solid var(--border);padding:20px 14px;position:sticky;top:0;height:100vh;overflow:auto;background:var(--panel);font-size:13px;}
#toc h3{margin:0 0 12px;color:var(--accent);font-size:14px;}
#toc a{display:block;color:var(--muted);text-decoration:none;padding:3px 6px;border-radius:5px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;}
#toc a:hover{color:var(--fg);background:#21262d;}
#toc a.h2{padding-left:6px;font-weight:600;color:#c9d1d9;margin-top:6px;}
#toc a.h3{padding-left:20px;}
#toc a.h4{padding-left:34px;font-size:12px;}
#content{flex:1;max-width:980px;margin:0 auto;padding:32px 48px 120px;overflow-x:hidden;}
#content h1{font-size:30px;border-bottom:2px solid var(--border);padding-bottom:12px;}
#content h2{font-size:23px;margin-top:42px;border-bottom:1px solid var(--border);padding-bottom:8px;color:var(--accent);}
#content h3{font-size:18px;margin-top:30px;color:#c9d1d9;}
#content h4{font-size:15px;margin-top:22px;color:var(--accent2);}
#content a{color:var(--accent);}
#content code{background:#1f2630;padding:2px 6px;border-radius:4px;font-family:"SF Mono",Menlo,Consolas,monospace;font-size:13px;}
#content pre{background:#161b22;border:1px solid var(--border);border-radius:8px;padding:16px;overflow:auto;}
#content pre code{background:none;padding:0;font-size:13px;line-height:1.5;}
#content table{border-collapse:collapse;width:100%;margin:16px 0;font-size:14px;display:block;overflow:auto;}
#content th,#content td{border:1px solid var(--border);padding:8px 12px;text-align:left;}
#content th{background:#21262d;color:var(--accent);}
#content tr:nth-child(even){background:#11161d;}
#content blockquote{border-left:4px solid var(--accent);margin:16px 0;padding:6px 16px;background:#11161d;color:var(--muted);border-radius:0 6px 6px 0;}
.mermaid{background:#fff;border-radius:8px;padding:16px;margin:18px 0;text-align:center;overflow:auto;}
#content hr{border:none;border-top:1px solid var(--border);margin:32px 0;}
::-webkit-scrollbar{width:10px;height:10px;}::-webkit-scrollbar-thumb{background:#30363d;border-radius:5px;}
#top{position:fixed;right:24px;bottom:24px;background:var(--accent);color:#0d1117;border:none;border-radius:50%;width:46px;height:46px;font-size:20px;cursor:pointer;box-shadow:0 4px 12px rgba(0,0,0,.4);}
</style>
</head>
<body>
<div id="layout">
  <nav id="toc"><h3>目录</h3><div id="toclist"></div></nav>
  <main id="content"></main>
</div>
<button id="top" onclick="window.scrollTo({top:0,behavior:'smooth'})">↑</button>
<script>
const MD = decodeURIComponent(escape(atob("__B64__")));
const renderer = new marked.Renderer();
const origCode = renderer.code.bind(renderer);
renderer.code = function(code, lang){
  const text = (typeof code==='object') ? code.text : code;
  const language = (typeof code==='object') ? code.lang : lang;
  if(language==='mermaid'){ return '<div class="mermaid">'+text.replace(/</g,'&lt;')+'</div>'; }
  return origCode(code, lang);
};
marked.setOptions({renderer, gfm:true, breaks:false});
document.getElementById('content').innerHTML = marked.parse(MD);
// 代码高亮
document.querySelectorAll('#content pre code').forEach(b=>{ try{hljs.highlightElement(b);}catch(e){} });
// 构建 TOC + 锚点
const toc=document.getElementById('toclist');
document.querySelectorAll('#content h1,#content h2,#content h3,#content h4').forEach((h,i)=>{
  const id='h'+i; h.id=id;
  const a=document.createElement('a'); a.href='#'+id; a.textContent=h.textContent;
  a.className=h.tagName.toLowerCase(); toc.appendChild(a);
});
// mermaid 渲染
mermaid.initialize({startOnLoad:false, theme:'default', securityLevel:'loose', flowchart:{useMaxWidth:true}, sequence:{useMaxWidth:true}});
mermaid.run({querySelector:'.mermaid'}).catch(e=>console.error('mermaid',e));
</script>
</body>
</html>"""
    html = html.replace("__TITLE__", title).replace("__B64__", b64)
    pathlib.Path(dst).write_text(html, encoding="utf-8")
    print(f"OK -> {dst} ({len(html)} bytes, md {len(md)} chars)")

if __name__ == "__main__":
    main()
