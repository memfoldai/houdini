// One self-contained HTML file: no network, no fonts to fetch, no framework.
// The story mechanic (segmented progress, tap zones, hold-to-pause) is driven
// by a single requestAnimationFrame loop, which the research flagged as the
// reason pause/resume/skip stay trivial where CSS animations would fight back.
//
// Typography is sized in container-query units (cqmin) against the reel, NOT
// viewport units. On a desktop the 9:16 reel is far narrower than the window,
// so vw-based type overflowed the card — cqmin ties every size to the card
// itself, so a card looks identical at any window size.

const esc = (s) =>
  String(s ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");

// Loud duotone blocks, Spotify-adjacent but deliberately not Spotify green.
// Each palette is {bg, ink, accent}: ink is the high-contrast hero/body colour
// picked to stay legible on bg; accent is the decorative pop (eyebrow, progress
// fill, underlines) and never carries body text.
const LOUD = [
  { bg: "#FF2D9B", ink: "#0A0A0A", accent: "#D6FF3D" },
  { bg: "#3D5AFE", ink: "#FFFFFF", accent: "#FFD23D" },
  { bg: "#7B2FF7", ink: "#FFFFFF", accent: "#22E4DB" },
  { bg: "#FF7A00", ink: "#160522", accent: "#3D5AFE" },
  { bg: "#22E4DB", ink: "#07211F", accent: "#FF2D9B" },
  { bg: "#0A0A0A", ink: "#FAF7F0", accent: "#D6FF3D" },
];
const TITLE = { bg: "#12071F", ink: "#FAF7F0", accent: "#FF2D9B" };
const DRAMA = { bg: "#0A0A0A", ink: "#FAF7F0", accent: "#FF2D9B" };
const GOLD = { bg: "#1A0B2E", ink: "#FAF7F0", accent: "#FFC94D" };
const PAPER = { bg: "#FAF7F0", ink: "#0A0A0A", accent: "#FF2D9B" };

function palette(card, loudCounter) {
  if (card.kind === "title") return TITLE;
  if (card.kind === "reveal") return DRAMA;
  if (card.kind === "trophy") return GOLD;
  if (card.kind === "summary") return PAPER;
  return LOUD[loudCounter % LOUD.length];
}

function hero(card) {
  if (card.heroNumber !== undefined) {
    const unit = card.heroUnit ? `<span class="unit">${esc(card.heroUnit)}</span>` : "";
    return `<div class="hero"><span class="num" data-to="${card.heroNumber}">0</span>${unit}</div>`;
  }
  if (card.heroText !== undefined) return `<div class="hero hero-text">${esc(card.heroText)}</div>`;
  return "";
}

function body(card) {
  switch (card.kind) {
    case "title":
      return `<div class="wordmark">WRAPPED</div>`;
    case "ranked": {
      const rows = card.rows
        .map(
          (r) =>
            `<li><div class="rk-head"><span class="rk-label">${esc(r.label)}</span><span class="rk-pct"><span class="num" data-to="${r.pct}">0</span>%</span></div><div class="rk-track"><i style="--w:${r.pct}%"></i></div></li>`,
        )
        .join("");
      return `<ul class="ranked">${rows}</ul>`;
    }
    case "podium": {
      const rows = card.rows
        .map(
          (r) =>
            `<li style="--i:${r.rank}"><span class="rank">${r.rank}</span><span class="who">${esc(r.name)}</span><span class="val">${esc(r.value)}<small>${esc(r.unit)}</small></span></li>`,
        )
        .join("");
      return `<ol class="podium">${rows}</ol>`;
    }
    case "person":
      return `<div class="badge">${esc(card.badge)}</div><div class="who-big">${esc(card.personName)}</div>`;
    case "trophy": {
      const podium =
        card.winner && card.runnersUp && card.runnersUp.length
          ? `<ol class="trophy-runners">${card.runnersUp
              .map((r, i) => `<li><span class="tr-medal">${["🥈", "🥉"][i] ?? i + 2}</span><span class="tr-name">${esc(r.name)}</span><span class="tr-val">${esc(r.value)}</span></li>`)
              .join("")}</ol>`
          : "";
      const win = card.winner
        ? `<div class="trophy-win reveal-target">${esc(card.winner)}</div><div class="trophy-stat">${esc(card.stat)}</div>${podium}`
        : `<div class="trophy-stat empty">${esc(card.stat)}</div>`;
      return `<div class="trophy-emoji">${esc(card.emoji)}</div><div class="trophy-title">${esc(card.title)}</div>${win}`;
    }
    case "summary": {
      const cells = card.grid
        .map((g) => `<div class="cell"><div class="cell-label">${esc(g.label)}</div><div class="cell-val">${esc(g.value)}</div></div>`)
        .join("");
      return `<div class="grid">${cells}</div><button class="share" type="button" aria-label="Download this card as an image">save the card</button>`;
    }
    default:
      return "";
  }
}

function section(card, idx, pal) {
  const kicker = card.kicker ? `<div class="kicker">${esc(card.kicker)}</div>` : "";
  const heroHtml = card.kind === "reveal" ? `<div class="hero hero-text reveal-target">${esc(card.heroText)}</div>` : hero(card);
  const bodyHtml = body(card);
  const sub = card.sub || card.line ? `<p class="sub">${esc(card.sub ?? card.line)}</p>` : "";
  const style = `--bg:${pal.bg};--ink:${pal.ink};--accent:${pal.accent}`;
  // Title and trophy read best hero-first; the rest lead with the eyebrow.
  const inner =
    card.kind === "title" || card.kind === "trophy"
      ? `${bodyHtml}${kicker}${heroHtml}${sub}`
      : `${kicker}${heroHtml}${bodyHtml}${sub}`;
  return `<section class="card ${esc(card.kind)}${idx === 0 ? " active" : ""}" style="${style}" data-idx="${idx}" aria-hidden="${idx === 0 ? "false" : "true"}">
      <div class="card-inner">${inner}</div>
    </section>`;
}

export function renderHtml(cards, meta = {}) {
  const team = meta.team ?? "Houdini";
  const week = meta.weekLabel ?? "";
  let loud = 0;
  const sections = cards
    .map((c, i) => {
      const pal = palette(c, loud);
      if (!["title", "reveal", "trophy", "summary"].includes(c.kind)) loud++;
      return section(c, i, pal);
    })
    .join("\n");

  const segs = cards.map((_, i) => `<span class="seg"${i === 0 ? ' data-on="1"' : ""}><i></i></span>`).join("");
  const title = esc(`${team} Wrapped${week ? ` · ${week}` : ""}`.trim());

  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<title>${title}</title>
<style>${CSS}</style>
</head>
<body>
<div class="stage" role="group" aria-roledescription="story" aria-label="${esc(team)} Wrapped">
  <div class="progress" aria-hidden="true">${segs}</div>
  <div class="reel">${sections}<canvas class="confetti" aria-hidden="true"></canvas></div>
  <button class="nav prev" type="button" aria-label="Previous card"></button>
  <button class="nav next" type="button" aria-label="Next card"></button>
  <div class="hint" aria-hidden="true">tap · hold to pause · ← →</div>
</div>
<canvas id="shot" width="1080" height="1920" hidden></canvas>
<script>
const DATA = ${JSON.stringify({ team, week })};
${JS}
</script>
</body>
</html>`;
}

const CSS = `
:root{color-scheme:dark light}
*{margin:0;padding:0;box-sizing:border-box}
html,body{height:100%;background:#000;overflow:hidden}
body{font-family:"Helvetica Neue",Helvetica,"Segoe UI",Roboto,system-ui,Arial,sans-serif;-webkit-font-smoothing:antialiased}
/* Words wrap at spaces only; a single over-long token may break, but nothing
   hyphenates mid-word. */
p,div,span,li,ol,button{overflow-wrap:break-word;word-break:normal;hyphens:none}
.stage{position:fixed;inset:0;display:grid;place-items:center;background:#000}
/* container-type:size makes the reel a query container, so cqmin below is a
   percentage of THIS box, not the window. */
.reel{position:relative;container-type:size;width:min(100vw,calc(100vh * 9 / 16));height:min(100vh,calc(100vw * 16 / 9));overflow:hidden;border-radius:min(2.2vh,20px)}
.card{position:absolute;inset:0;background:var(--bg);color:var(--ink);display:grid;opacity:0;visibility:hidden;transition:opacity .35s ease}
.card.active{opacity:1;visibility:visible}
.card-inner{align-self:center;width:100%;max-height:100%;overflow-y:auto;padding:8cqmin 7cqmin;display:flex;flex-direction:column;gap:4.4cqmin;scrollbar-width:none}
.card-inner::-webkit-scrollbar{display:none}
.kicker{font-size:3.4cqmin;font-weight:800;letter-spacing:.16em;text-transform:uppercase;color:var(--accent);text-wrap:balance}
.hero{font-weight:900;line-height:.92;letter-spacing:-.03em;font-size:23cqmin;text-wrap:balance}
.hero .unit{display:block;font-size:6.4cqmin;font-weight:800;letter-spacing:0;opacity:.85;margin-top:.35em}
.hero-text{font-size:15cqmin;line-height:.98}
.sub{font-size:5cqmin;line-height:1.35;font-weight:600;text-wrap:pretty;opacity:.96}
.num,.unit,.val,.cell-val,.bar-pct{font-variant-numeric:tabular-nums;font-feature-settings:"tnum"}
.wordmark{font-size:9cqmin;font-weight:900;letter-spacing:.32em;color:var(--accent);text-transform:uppercase}
/* ranked breakdown (categories) */
.ranked{list-style:none;display:flex;flex-direction:column;gap:3.2cqmin;margin-top:1cqmin}
.ranked li{display:flex;flex-direction:column;gap:1.4cqmin}
.rk-head{display:flex;align-items:baseline;justify-content:space-between;gap:1ch;font-weight:800}
.rk-label{font-size:4.6cqmin;letter-spacing:.01em;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.rk-pct{font-size:5.8cqmin;flex:none}
.rk-track{height:3.4cqmin;border-radius:999px;background:color-mix(in srgb,var(--ink) 16%,transparent);overflow:hidden}
.rk-track i{display:block;height:100%;width:0;border-radius:999px;background:var(--accent);transition:width .9s cubic-bezier(.2,.8,.2,1)}
.card.active .rk-track i{width:var(--w)}
/* podium */
.podium{list-style:none;display:flex;flex-direction:column;gap:1.7cqmin}
.podium li{display:flex;align-items:center;gap:2.2cqmin;font-weight:800;font-size:5.4cqmin;opacity:0;transform:translateY(8px);transition:opacity .45s ease,transform .45s ease;transition-delay:calc(var(--i) * .05s)}
.card.active .podium li{opacity:1;transform:none}
.podium .rank{width:1.5em;color:var(--accent);flex:none;font-variant-numeric:tabular-nums}
.podium .who{flex:1;min-width:0;letter-spacing:-.01em;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.podium li:first-child{font-size:7cqmin}
.podium li:nth-child(1) .rank{color:#FFC94D}
.podium .val{flex:none}
.podium .val small{font-size:.55em;font-weight:700;margin-left:.15ch;opacity:.72}
/* person / superlative */
.badge{font-weight:900;font-size:14cqmin;line-height:.96;letter-spacing:-.02em;color:var(--accent);text-wrap:balance}
.who-big{font-weight:800;font-size:6.4cqmin;opacity:.95}
/* trophy */
.trophy-emoji{font-size:16cqmin;line-height:1}
.trophy-title{font-weight:900;font-size:8cqmin;line-height:1.02;letter-spacing:-.02em;text-wrap:balance}
.trophy-win{font-weight:900;font-size:15cqmin;line-height:.94;color:var(--accent);text-wrap:balance}
.trophy-stat{font-size:5cqmin;font-weight:600;line-height:1.35;text-wrap:pretty}
.trophy-stat.empty{opacity:.85;font-style:italic}
.trophy-runners{list-style:none;display:flex;flex-direction:column;gap:1.7cqmin;margin-top:1.8cqmin;width:100%}
.trophy-runners li{display:flex;align-items:center;gap:1.8cqmin;font-weight:800;font-size:4.8cqmin;opacity:.9}
.tr-medal{width:1.6em;flex:none;text-align:center;color:var(--accent);font-variant-numeric:tabular-nums}
.tr-name{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.tr-val{flex:none;opacity:.75;font-variant-numeric:tabular-nums}
/* summary */
.grid{display:grid;grid-template-columns:1fr 1fr;gap:3cqmin}
.cell{background:color-mix(in srgb,var(--ink) 7%,transparent);border:1px solid color-mix(in srgb,var(--ink) 16%,transparent);border-radius:3cqmin;padding:3.4cqmin}
.cell-label{font-size:3.2cqmin;font-weight:800;letter-spacing:.1em;text-transform:uppercase;opacity:.6}
.cell-val{font-size:6.4cqmin;font-weight:900;letter-spacing:-.02em;margin-top:.2em;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.share{position:relative;z-index:6;margin-top:1cqmin;align-self:start;font:inherit;font-weight:800;font-size:4.4cqmin;padding:.7em 1.3em;border:none;border-radius:999px;background:var(--accent);color:var(--bg);cursor:pointer}
.share:focus-visible{outline:3px solid var(--ink);outline-offset:3px}
/* chrome */
.progress{position:absolute;top:0;left:0;right:0;z-index:5;display:flex;gap:5px;padding:12px 12px 0}
.seg{flex:1;height:3px;border-radius:999px;background:rgba(255,255,255,.32);overflow:hidden}
.seg i{display:block;height:100%;width:0;background:#fff}
.seg[data-done] i{width:100%}
.nav{position:absolute;top:0;bottom:0;width:32%;border:none;background:transparent;cursor:pointer;z-index:4;-webkit-tap-highlight-color:transparent}
.nav.prev{left:0}
.nav.next{right:0;width:68%}
.nav:focus-visible{outline:3px solid #fff;outline-offset:-6px}
.confetti{position:absolute;inset:0;z-index:7;pointer-events:none}
.hint{position:absolute;bottom:max(10px,env(safe-area-inset-bottom));left:0;right:0;text-align:center;z-index:5;font-size:.72rem;font-weight:700;letter-spacing:.14em;text-transform:uppercase;color:rgba(255,255,255,.55);pointer-events:none;mix-blend-mode:difference}
/* title flourish */
.card.title{background:radial-gradient(125% 90% at 50% 26%, #2c1150 0%, var(--bg) 62%)}
.wordmark{text-shadow:0 0 40px color-mix(in srgb,var(--accent) 45%, transparent)}
.card.title.active .wordmark{animation:rise .7s cubic-bezier(.2,.9,.3,1.25) both}
.card.title.active .hero{animation:rise .7s .08s cubic-bezier(.2,.9,.3,1.25) both}
.card.title.active .kicker{animation:rise .6s .18s ease both}
.card.title.active .sub{animation:rise .6s .26s ease both}
@keyframes rise{from{opacity:0;transform:translateY(26px) scale(.95)}to{opacity:1;transform:none}}
/* trophy spotlight */
.card.trophy{background:radial-gradient(circle at 50% 24%, color-mix(in srgb,var(--accent) 20%, var(--bg)), var(--bg) 58%)}
.trophy-emoji{filter:drop-shadow(0 8px 26px color-mix(in srgb,var(--accent) 55%, transparent))}
.trophy-win{text-shadow:0 0 34px color-mix(in srgb,var(--accent) 55%, transparent)}
/* summary finale */
.card.summary{background:linear-gradient(155deg,#FBF3EA,#FBE7F1)}
.summary .cell-val{color:var(--accent)}
.summary .cell{border-color:color-mix(in srgb,var(--accent) 32%, transparent)}
@media (prefers-reduced-motion: reduce){
  .card{transition:none}
  .rk-track i,.podium li{transition:none}
  .podium li{opacity:1;transform:none}
  .card.title.active .wordmark,.card.title.active .hero,.card.title.active .kicker,.card.title.active .sub{animation:none}
}
`;

const JS = `
(function(){
  var cards=[].slice.call(document.querySelectorAll('.card'));
  var segs=[].slice.call(document.querySelectorAll('.seg'));
  var prevBtn=document.querySelector('.nav.prev');
  var nextBtn=document.querySelector('.nav.next');
  var reduce=matchMedia('(prefers-reduced-motion: reduce)').matches;
  var DUR_BASE=4200, DUR_PER_CHAR=22, MAXDUR=9000;
  var idx=0, paused=false, done=false, start=0, raf=0, holdTimer=0, held=false, pausedAt=0;

  // Confetti on the celebratory cards (title, the reveal, the trophies, the
  // finale). Canvas is scoped to the reel and pointer-events:none, so it rains
  // over the content without blocking taps. Skipped under reduced motion.
  var cfReel=document.querySelector('.reel'), cfc=document.querySelector('.confetti'), cfx=cfc.getContext('2d'), cfp=[], cfraf=0;
  var CF_COLORS=['#FF2D9B','#D6FF3D','#22E4DB','#FFC94D','#3D5AFE','#FF7A00'];
  function cfSize(){ var r=cfReel.getBoundingClientRect(); cfc.width=Math.round(r.width)||360; cfc.height=Math.round(r.height)||640; }
  function burst(){
    if(reduce) return;
    cfSize();
    var W=cfc.width;
    for(var i=0;i<130;i++) cfp.push({x:Math.random()*W, y:-20-Math.random()*140, vx:(Math.random()-0.5)*3.2, vy:2+Math.random()*3.6, w:5+Math.random()*6, h:3+Math.random()*5, c:CF_COLORS[(Math.random()*CF_COLORS.length)|0], a:1, rot:Math.random()*6.28, vr:(Math.random()-0.5)*0.34});
    if(!cfraf) cfraf=requestAnimationFrame(cfStep);
  }
  function cfStep(){
    cfx.clearRect(0,0,cfc.width,cfc.height);
    for(var i=cfp.length-1;i>=0;i--){ var p=cfp[i];
      p.x+=p.vx; p.y+=p.vy; p.vy+=0.05; p.rot+=p.vr; p.a-=0.005;
      if(p.y>cfc.height+30||p.a<=0){ cfp.splice(i,1); continue; }
      cfx.save(); cfx.globalAlpha=Math.max(0,p.a); cfx.translate(p.x,p.y); cfx.rotate(p.rot);
      cfx.fillStyle=p.c; cfx.fillRect(-p.w/2,-p.h/2,p.w,p.h); cfx.restore();
    }
    if(cfp.length) cfraf=requestAnimationFrame(cfStep); else { cfraf=0; cfx.clearRect(0,0,cfc.width,cfc.height); }
  }
  function maybeConfetti(card){ if(/(^| )(title|reveal|trophy|summary)( |$)/.test(card.className)) burst(); }

  function durationFor(card){
    var txt=(card.textContent||'').trim().length;
    return Math.min(MAXDUR, DUR_BASE + txt*DUR_PER_CHAR);
  }
  function countUp(card){
    card.querySelectorAll('.num').forEach(function(el){
      var to=parseFloat(el.getAttribute('data-to'))||0;
      if(reduce){ el.textContent=to.toLocaleString('en-US'); return; }
      var t0=performance.now(), D=900;
      (function step(now){
        var k=Math.min(1,(now-t0)/D);
        var e=1-Math.pow(1-k,3);
        el.textContent=Math.round(to*e).toLocaleString('en-US');
        if(k<1) requestAnimationFrame(step);
      })(performance.now());
    });
    var rt=card.querySelector('.reveal-target');
    if(rt && !reduce){ rt.style.filter='blur(16px)'; rt.style.opacity='0'; rt.getBoundingClientRect();
      rt.style.transition='filter .7s ease, opacity .7s ease'; rt.style.filter='blur(0)'; rt.style.opacity='1'; }
  }
  function paint(){
    segs.forEach(function(s,i){ s.removeAttribute('data-done'); s.querySelector('i').style.width = i<idx?'100%':'0'; if(i<idx) s.setAttribute('data-done',''); });
  }
  function show(n){
    n=Math.max(0,Math.min(cards.length-1,n));
    cards[idx].classList.remove('active'); cards[idx].setAttribute('aria-hidden','true');
    idx=n; done=false;
    cards[idx].classList.add('active'); cards[idx].setAttribute('aria-hidden','false');
    cards[idx].querySelector('.card-inner').scrollTop=0;
    paint(); countUp(cards[idx]); maybeConfetti(cards[idx]); start=performance.now();
  }
  function next(){ if(idx>=cards.length-1){ finish(); } else show(idx+1); }
  function prev(){ show(idx-1); }
  function finish(){ done=true; segs.forEach(function(s){ s.querySelector('i').style.width='100%'; }); }

  function loop(now){
    raf=requestAnimationFrame(loop);
    if(reduce||paused||done) return;
    var dur=durationFor(cards[idx]);
    var k=(now-start)/dur;
    var fill=segs[idx]?segs[idx].querySelector('i'):null;
    if(fill) fill.style.width=Math.min(100,k*100)+'%';
    if(k>=1) next();
  }

  // Hold-to-pause vs quick-tap-to-navigate: a press pauses immediately so text
  // can be read; a release under 200ms is a tap and advances. On resume the
  // start shifts by the paused span so the bar continues instead of rewinding.
  function down(){ held=false; paused=true; pausedAt=performance.now(); holdTimer=setTimeout(function(){ held=true; }, 200); }
  function resume(){ paused=false; start+=performance.now()-pausedAt; }
  function up(zone){ return function(){ clearTimeout(holdTimer); var wasHeld=held; resume();
    if(!wasHeld){ if(zone==='prev') prev(); else next(); } }; }
  prevBtn.addEventListener('pointerdown',down); prevBtn.addEventListener('pointerup',up('prev')); prevBtn.addEventListener('pointercancel',function(){clearTimeout(holdTimer);resume();});
  nextBtn.addEventListener('pointerdown',down); nextBtn.addEventListener('pointerup',up('next')); nextBtn.addEventListener('pointercancel',function(){clearTimeout(holdTimer);resume();});

  document.addEventListener('keydown',function(e){
    if(e.key==='ArrowRight'||e.key===' '){ e.preventDefault(); next(); }
    else if(e.key==='ArrowLeft'){ e.preventDefault(); prev(); }
  });

  var shareBtn=document.querySelector('.share');
  if(shareBtn) shareBtn.addEventListener('click',function(e){ e.stopPropagation(); drawShot(); });

  function drawShot(){
    var c=document.getElementById('shot'), x=c.getContext('2d'), W=c.width, H=c.height;
    var g=x.createLinearGradient(0,0,W,H); g.addColorStop(0,'#12071F'); g.addColorStop(1,'#3D1152');
    x.fillStyle=g; x.fillRect(0,0,W,H);
    x.textAlign='left';
    x.fillStyle='#FF2D9B'; x.font='700 46px "Helvetica Neue",Arial,sans-serif';
    x.fillText((DATA.week||'').toUpperCase(), 90, 200);
    x.fillStyle='#FAF7F0'; x.font='900 130px "Helvetica Neue",Arial,sans-serif';
    wrap(x, DATA.team+' WRAPPED', 90, 340, W-180, 130);
    var cells=[].slice.call(document.querySelectorAll('.summary .cell'));
    var y=780, col=0;
    cells.forEach(function(cell){
      var lx=90+col*((W-180)/2)+(col?40:0);
      x.fillStyle='rgba(250,247,240,.55)'; x.font='800 34px "Helvetica Neue",Arial,sans-serif';
      x.fillText(cell.querySelector('.cell-label').textContent.toUpperCase(), lx, y);
      x.fillStyle='#FAF7F0'; x.font='900 72px "Helvetica Neue",Arial,sans-serif';
      x.fillText(cell.querySelector('.cell-val').textContent, lx, y+80);
      col++; if(col===2){ col=0; y+=200; }
    });
    x.fillStyle='#FFC94D'; x.textAlign='center'; x.font='800 40px "Helvetica Neue",Arial,sans-serif';
    x.fillText('HOUDINI WRAPPED', W/2, H-140);
    var a=document.createElement('a');
    a.href=c.toDataURL('image/png'); a.download=(DATA.team||'houdini').replace(/\\s+/g,'-').toLowerCase()+'-wrapped.png'; a.click();
  }
  function wrap(x,text,px,py,maxw,lh){
    var words=String(text).split(' '), line='', yy=py;
    for(var i=0;i<words.length;i++){ var t=line+words[i]+' '; if(x.measureText(t).width>maxw && i>0){ x.fillText(line,px,yy); line=words[i]+' '; yy+=lh; } else line=t; }
    x.fillText(line,px,yy);
  }

  // Deep-link: #N opens directly on card N (used for previews and sharing a
  // single card's spot in the reel).
  var startAt=parseInt((location.hash||'').slice(1),10);
  if(startAt>0 && startAt<cards.length){ cards[0].classList.remove('active'); cards[0].setAttribute('aria-hidden','true');
    idx=startAt; cards[idx].classList.add('active'); cards[idx].setAttribute('aria-hidden','false'); }
  paint(); countUp(cards[idx]); start=performance.now(); requestAnimationFrame(loop);
  setTimeout(function(){ maybeConfetti(cards[idx]); }, 120);
})();
`;
