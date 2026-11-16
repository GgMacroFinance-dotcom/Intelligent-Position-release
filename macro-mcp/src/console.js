// Browser test console served at GET / (text/html). Same-origin only; every data call carries the viewer's key.
export const CONSOLE_CSP = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

export const CONSOLE_HTML = `<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>WhichAPI Macro</title>
<style>
:root{--bg:#fff;--fg:#171717;--muted:#626262;--faint:#8a8a8a;--line:#e8e8e8;--soft:#f6f6f6;--ok:#15803d;--err:#b91c1c;--accent:#171717;--mono:ui-monospace,SFMono-Regular,Consolas,monospace;--sans:system-ui,-apple-system,"Segoe UI","Microsoft YaHei",sans-serif}
@media(prefers-color-scheme:dark){:root{--bg:#111;--fg:#ededed;--muted:#a3a3a3;--faint:#7a7a7a;--line:#2a2a2a;--soft:#1a1a1a;--ok:#4ade80;--err:#f87171;--accent:#ededed}}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:14px/1.6 var(--sans)}
main{max-width:1100px;margin:0 auto;padding:32px 20px 64px}
h1{font-size:24px;margin:0 0 4px;letter-spacing:-.4px}h2{font-size:16px;margin:36px 0 12px}
p{margin:0;color:var(--muted)}code,pre,.mono{font-family:var(--mono);font-size:12.5px}
.row{display:flex;gap:10px;flex-wrap:wrap;align-items:center}
input,select,textarea{font:inherit;color:inherit;background:var(--bg);border:1px solid var(--line);border-radius:8px;padding:8px 10px;min-width:0}
input{flex:1 1 280px}textarea{width:100%;min-height:96px;font-family:var(--mono);font-size:12.5px}
button{font:inherit;cursor:pointer;border-radius:999px;padding:8px 16px;border:1px solid var(--accent);background:var(--accent);color:var(--bg)}
button.ghost{background:transparent;color:var(--fg);border-color:var(--line)}button:disabled{opacity:.5;cursor:default}
.note{font-size:12px;color:var(--faint);margin-top:6px}.err{color:var(--err)}.ok{color:var(--ok)}
.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(240px,1fr));gap:12px}
.card{border:1px solid var(--line);border-radius:12px;padding:16px;display:flex;flex-direction:column;gap:6px;min-width:0}
.card .label{font-weight:600;font-size:13px}.card .src{font-size:11px;color:var(--faint)}
.card .val{font:600 24px/1.2 var(--mono);letter-spacing:-.4px}.card .val span{font:400 12px var(--sans);color:var(--faint);margin-left:6px;letter-spacing:0}.card .sub{font:12px var(--mono);color:var(--faint)}
.wide{grid-column:1/-1}.docs{margin:0;padding-left:18px;display:flex;flex-direction:column;gap:6px}.docs a{color:inherit}
pre{background:var(--soft);border:1px solid var(--line);border-radius:8px;padding:14px;overflow:auto;max-height:420px;margin:12px 0 0;white-space:pre-wrap;word-break:break-word}
.warn{font-size:12px;color:var(--faint);margin-top:12px}.warn li{margin:2px 0}
header{display:flex;justify-content:space-between;gap:16px;flex-wrap:wrap;align-items:flex-end;border-bottom:1px solid var(--line);padding-bottom:20px}
</style>
</head>
<body>
<main>
<header>
  <div><h1>WhichAPI Macro</h1><p>美国宏观数据（纽约联储、FRED、财政部、TIC、Federal Register、SEC）的 MCP 服务与 HTTP API 测试页。FRED 序列经 WhichAPI 托管服务取，要在服务端设 WHICHAPI_API_KEY。</p></div>
  <span class="mono" id="status">检查服务…</span>
</header>

<h2>访问密钥</h2>
<div class="row"><input id="key" type="password" autocomplete="off" placeholder="粘贴 MACRO_API_KEY"><button id="save">保存</button><button class="ghost" id="clear">清除</button></div>
<div class="note">只保存在这个浏览器里，用于请求头 Authorization: Bearer。</div>

<h2>宏观快照</h2>
<div class="row"><button id="load">加载快照</button><label class="row note" style="margin:0"><input type="checkbox" id="fresh" style="flex:none"> 跳过缓存</label><span id="snapmeta" class="mono note" style="margin:0"></span></div>
<div id="snaperr" class="err note"></div>
<div class="grid" id="panels" style="margin-top:12px"></div>
<ul class="warn" id="warnings"></ul>

<h2>调用单个工具</h2>
<div class="row"><select id="tool"><option>先保存密钥</option></select><button id="run" disabled>运行</button></div>
<p class="note" id="tooldesc"></p>
<textarea id="args" spellcheck="false">{}</textarea>
<pre id="out" hidden></pre>

<h2>接入 MCP 客户端</h2>
<p>Claude Code：</p>
<pre id="mcpcmd"></pre>
<p class="note">支持 Streamable HTTP 的客户端都可以用同样的地址和请求头。</p>
</main>
<script>
(function(){
  var $=function(id){return document.getElementById(id)};
  var store={get:function(){try{return localStorage.getItem('macroKey')||''}catch(e){return ''}},set:function(v){try{v?localStorage.setItem('macroKey',v):localStorage.removeItem('macroKey')}catch(e){}}};
  var examples={get_reference_rates:{rate:'EFFR',observations:5},get_fred_series:{series_id:'DGS10',limit:10},get_treasury_curve:{limit:3},get_tic_holdings:{country:'Japan',limit:6},get_debt_limit:{},search_federal_register:{term:'"Section 232"',order:'newest',limit:5},get_company_concept:{cik:'0001652044',concept:'PaymentsToAcquirePropertyPlantAndEquipment',limit:6},macro_snapshot:{panels:['sofr','ust_10y']},list_macro_panels:{}};
  var tools=[];
  $('key').value=store.get();
  function mcp(){var k=$('key').value?'<你的密钥>':'<MACRO_API_KEY>';$('mcpcmd').textContent='claude mcp add --transport http whichapi-macro '+location.origin+'/mcp --header "Authorization: Bearer '+k+'"';}
  mcp();
  function headers(){var h={'Content-Type':'application/json'};var k=$('key').value.trim();if(k)h.Authorization='Bearer '+k;return h;}
  function api(path,body){return fetch(path,{method:body?'POST':'GET',headers:headers(),body:body?JSON.stringify(body):undefined}).then(function(r){return r.json().catch(function(){return {error:{code:'http_'+r.status,message:'HTTP '+r.status}}}).then(function(d){return {status:r.status,data:d}})});}
  fetch('/health').then(function(r){return r.json()}).then(function(d){$('status').textContent=d.server.name+' '+d.server.version+' · 在线';$('status').className='mono ok'}).catch(function(){$('status').textContent='服务无响应';$('status').className='mono err'});
  function fmt(v,u){if(v===null||v===undefined)return '—';if(u==='USD mn')return (v/1000).toLocaleString('en-US',{minimumFractionDigits:1,maximumFractionDigits:1})+' bn';var d=u==='CNY per USD'?4:2;return v.toLocaleString('en-US',{minimumFractionDigits:d,maximumFractionDigits:d});}
  function unit(u){return u==='USD mn'?'USD':u==='documents'?'':u;}
  function el(tag,cls,text){var e=document.createElement(tag);if(cls)e.className=cls;if(text!==undefined)e.textContent=text;return e;}
  function renderPanels(s){
    var box=$('panels');box.textContent='';
    s.panels.forEach(function(p){
      var c=el('div','card'+(p.documents?' wide':''));c.appendChild(el('div','label',p.label));c.appendChild(el('div','src',p.source));
      if(p.status==='error'){c.appendChild(el('div','err note',p.error.code+': '+p.error.message));}
      else if(p.documents){var ol=el('ol','docs');p.documents.forEach(function(d){var li=el('li');li.appendChild(el('span','mono note',(d.date||'')+'  '));if(d.url&&/^https:\\/\\//.test(d.url)){var a=el('a','',d.title||d.url);a.href=d.url;a.target='_blank';a.rel='noreferrer';li.appendChild(a);}else li.appendChild(document.createTextNode(d.title||''));ol.appendChild(li);});c.appendChild(ol);}
      else{var v=el('div','val',fmt(p.value,p.unit));if(unit(p.unit))v.appendChild(el('span','',unit(p.unit)));c.appendChild(v);
        var sub='截至 '+(p.as_of||'—');if(p.change!==null&&p.previous)sub+=' · '+(p.change>0?'+':'')+fmt(p.change,p.unit)+' vs '+p.previous.date;c.appendChild(el('div','sub',sub));
        if(p.extras&&p.extras.target_range)c.appendChild(el('div','sub','目标区间 '+p.extras.target_range[0]+'–'+p.extras.target_range[1]+'%'));
        if(p.extras&&typeof p.extras.slope_2s10s_pct_pts==='number')c.appendChild(el('div','sub','2s10s '+p.extras.slope_2s10s_pct_pts.toFixed(2)+' pts'));}
      box.appendChild(c);
    });
    var w=$('warnings');w.textContent='';(s.warnings||[]).forEach(function(t){w.appendChild(el('li','',t))});
  }
  $('load').onclick=function(){
    $('load').disabled=true;$('snaperr').textContent='';$('snapmeta').textContent='加载中…';
    api('/v1/snapshot'+($('fresh').checked?'?max_age_seconds=0':'')).then(function(r){
      if(!r.data.panels){$('snaperr').textContent=(r.data.error?r.data.error.code+': '+r.data.error.message:'HTTP '+r.status);$('snapmeta').textContent='';return;}
      renderPanels(r.data);$('snapmeta').textContent=r.data.status+' · '+r.data.generated_at;
    }).catch(function(e){$('snaperr').textContent=String(e)}).then(function(){$('load').disabled=false});
  };
  function loadTools(){
    api('/v1/tools').then(function(r){
      if(!r.data.tools){$('tooldesc').textContent=r.data.error?r.data.error.message:'无法读取工具列表';$('tooldesc').className='note err';return;}
      tools=r.data.tools;var s=$('tool');s.textContent='';tools.forEach(function(t){var o=el('option','',t.name);o.value=t.name;s.appendChild(o)});
      $('run').disabled=false;$('tooldesc').className='note';pick();
    });
  }
  function pick(){var t=tools.find(function(x){return x.name===$('tool').value});if(!t)return;$('tooldesc').textContent=t.description;$('args').value=JSON.stringify(examples[t.name]||{},null,2);}
  $('tool').onchange=pick;
  $('run').onclick=function(){
    var args;try{args=JSON.parse($('args').value||'{}')}catch(e){$('out').hidden=false;$('out').textContent='参数必须是 JSON 对象';return;}
    $('run').disabled=true;$('out').hidden=false;$('out').textContent='运行中…';
    api('/v1/tools/'+encodeURIComponent($('tool').value),args).then(function(r){$('out').textContent='HTTP '+r.status+'\\n'+JSON.stringify(r.data,null,2)}).catch(function(e){$('out').textContent=String(e)}).then(function(){$('run').disabled=false});
  };
  $('save').onclick=function(){store.set($('key').value.trim());mcp();loadTools();};
  $('clear').onclick=function(){$('key').value='';store.set('');mcp();};
  if($('key').value)loadTools();
})();
</script>
</body>
</html>`;
