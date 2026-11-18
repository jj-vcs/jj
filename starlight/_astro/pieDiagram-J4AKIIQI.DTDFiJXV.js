import{t as e}from"./chunk-JWPE2WC7.A3RzWITa.js";import{a as t}from"./mermaid.core.Bf0pvaT3.js";import{i as n,m as r}from"./chunk-6MUBBAK2.CfXUfaG5.js";import{C as i,H as a,U as o,_ as s,a as c,b as l,c as u,f as d,q as f,v as p}from"./chunk-BG4553SI.DzMjcvML.js";import{t as m}from"./ordinal.BAxfPAgN.js";import{m as h}from"./src.Duoub3vK.js";import{n as g}from"./chunk-Y2CYZVJY.DsF7k-Jl.js";import{t as _}from"./arc.cxkLdl1s.js";import{n as v}from"./mermaid-parser.core.CZlxp23a.js";import{t as y}from"./array.BifhSqXX.js";import{n as b}from"./path.fybaL0A-.js";import{m as x}from"./dist.B-59DSOZ.js";function S(e,t){return t<e?-1:t>e?1:t>=e?0:NaN}function C(e){return e}function w(){var e=C,t=S,n=null,r=b(0),i=b(x),a=b(0);function o(o){var s,c=(o=y(o)).length,l,u,d=0,f=Array(c),p=Array(c),m=+r.apply(this,arguments),h=Math.min(x,Math.max(-x,i.apply(this,arguments)-m)),g,_=Math.min(Math.abs(h)/c,a.apply(this,arguments)),v=_*(h<0?-1:1),b;for(s=0;s<c;++s)(b=p[f[s]=s]=+e(o[s],s,o))>0&&(d+=b);for(t==null?n!=null&&f.sort(function(e,t){return n(o[e],o[t])}):f.sort(function(e,n){return t(p[e],p[n])}),s=0,u=d?(h-c*v)/d:0;s<c;++s,m=g)l=f[s],b=p[l],g=m+(b>0?b*u:0)+v,p[l]={data:o[l],index:s,value:b,startAngle:m,endAngle:g,padAngle:_};return p}return o.value=function(t){return arguments.length?(e=typeof t==`function`?t:b(+t),o):e},o.sortValues=function(e){return arguments.length?(t=e,n=null,o):t},o.sort=function(e){return arguments.length?(n=e,t=null,o):n},o.startAngle=function(e){return arguments.length?(r=typeof e==`function`?e:b(+e),o):r},o.endAngle=function(e){return arguments.length?(i=typeof e==`function`?e:b(+e),o):i},o.padAngle=function(e){return arguments.length?(a=typeof e==`function`?e:b(+e),o):a},o}var T=d.pie,E={sections:new Map,showData:!1,config:T},D=E.sections,O=E.showData,k=structuredClone(T),A={getConfig:g(()=>structuredClone(k),`getConfig`),clear:g(()=>{D=new Map,O=E.showData,c()},`clear`),setDiagramTitle:f,getDiagramTitle:i,setAccTitle:o,getAccTitle:p,setAccDescription:a,getAccDescription:s,addSection:g(({label:e,value:t})=>{if(t<0)throw Error(`"${e}" has invalid value: ${t}. Negative values are not allowed in pie charts. All slice values must be >= 0.`);D.has(e)||(D.set(e,t),h.debug(`added new section: ${e}, with value: ${t}`))},`addSection`),getSections:g(()=>D,`getSections`),setShowData:g(e=>{O=e},`setShowData`),getShowData:g(()=>O,`getShowData`)},j=g((t,n)=>{e(t,n),n.setShowData(t.showData),t.sections.map(n.addSection)},`populateDb`),M={parse:g(async e=>{let t=await v(`pie`,e);h.debug(t),j(t,A)},`parse`)},N=g(e=>`
  .pieCircle{
    stroke: ${e.pieStrokeColor};
    stroke-width : ${e.pieStrokeWidth};
    opacity : ${e.pieOpacity};
  }
  .pieCircle.highlighted{
    scale: 1.05;
    opacity: 1;
  }
  .pieCircle.highlightedOnHover:hover{
    transition-duration: 250ms;
    scale: 1.05;
    opacity: 1;
  }
  .pieOuterCircle{
    stroke: ${e.pieOuterStrokeColor};
    stroke-width: ${e.pieOuterStrokeWidth};
    fill: none;
  }
  .pieTitleText {
    text-anchor: middle;
    font-size: ${e.pieTitleTextSize};
    fill: ${e.pieTitleTextColor};
    font-family: ${e.fontFamily};
  }
  .slice {
    font-family: ${e.fontFamily};
    fill: ${e.pieSectionTextColor};
    font-size:${e.pieSectionTextSize};
    // fill: white;
  }
  .legend text {
    fill: ${e.pieLegendTextColor};
    font-family: ${e.fontFamily};
    font-size: ${e.pieLegendTextSize};
  }
`,`getStyles`),P=g(e=>{let t=[...e.values()].reduce((e,t)=>e+t,0),n=[...e.entries()].map(([e,t])=>({label:e,value:t})).filter(e=>e.value/t*100>=1);return w().value(e=>e.value).sort(null)(n)},`createPieArcs`),F={parser:M,db:A,renderer:{draw:g((e,i,a,o)=>{h.debug(`rendering pie chart
`+e);let s=o.db,c=l(),d=n(s.getConfig(),c.pie),f=t(i),p=f.append(`g`);p.attr(`transform`,`translate(225,225)`);let{themeVariables:g}=c,[v]=r(g.pieOuterStrokeWidth);v??=2;let y=d.legendPosition,b=d.textPosition,x=d.donutHole>0&&d.donutHole<=.9?d.donutHole:0,S=_().innerRadius(x*185).outerRadius(185),C=_().innerRadius(185*b).outerRadius(185*b),w=p.append(`g`);w.append(`circle`).attr(`cx`,0).attr(`cy`,0).attr(`r`,185+v/2).attr(`class`,`pieOuterCircle`);let T=s.getSections(),E=P(T),D=[g.pie1,g.pie2,g.pie3,g.pie4,g.pie5,g.pie6,g.pie7,g.pie8,g.pie9,g.pie10,g.pie11,g.pie12],O=0;T.forEach(e=>{O+=e});let k=E.filter(e=>(e.data.value/O*100).toFixed(0)!==`0`),A=m(D).domain([...T.keys()]);w.selectAll(`mySlices`).data(k).enter().append(`path`).attr(`d`,S).attr(`fill`,e=>A(e.data.label)).attr(`class`,e=>{let t=`pieCircle`;return d.highlightSlice===`hover`?t+=` highlightedOnHover`:d.highlightSlice===e.data.label&&(t+=` highlighted`),t}),w.selectAll(`mySlices`).data(k).enter().append(`text`).text(e=>(e.data.value/O*100).toFixed(0)+`%`).attr(`transform`,e=>`translate(`+C.centroid(e)+`)`).style(`text-anchor`,`middle`).attr(`class`,`slice`);let j=p.append(`text`).text(s.getDiagramTitle()).attr(`x`,0).attr(`y`,-200).attr(`class`,`pieTitleText`),M=[...T.entries()].map(([e,t])=>({label:e,value:t})),N=p.selectAll(`.legend`).data(M).enter().append(`g`).attr(`class`,`legend`);N.append(`rect`).attr(`width`,18).attr(`height`,18).style(`fill`,e=>A(e.label)).style(`stroke`,e=>A(e.label)),N.append(`text`).attr(`x`,22).attr(`y`,14).text(e=>s.getShowData()?`${e.label} [${e.value}]`:e.label);let F=Math.max(...N.selectAll(`text`).nodes().map(e=>e?.getBoundingClientRect().width??0)),I=450,L=490,R=M.length*22;switch(y){case`center`:N.attr(`transform`,(e,t)=>{let n=22*M.length/2,r=-F/2-22,i=t*22-n;return`translate(`+r+`,`+i+`)`});break;case`top`:I+=R,N.attr(`transform`,(e,t)=>`translate(${-F/2-22}, ${t*22-185})`),w.attr(`transform`,()=>`translate(0, ${R+22})`);break;case`bottom`:I+=R,N.attr(`transform`,(e,t)=>{let n=-F/2-22,r=t*22- -207;return`translate(`+n+`,`+r+`)`});break;case`left`:L+=22+F,N.attr(`transform`,(e,t)=>{let n=22*M.length/2;return`translate(-207,`+(t*22-n)+`)`}),w.attr(`transform`,()=>`translate(${F+18+4}, 0)`);break;default:L+=22+F,N.attr(`transform`,(e,t)=>{let n=22*M.length/2;return`translate(216,`+(t*22-n)+`)`})}let z=j.node()?.getBoundingClientRect().width??0,B=225-z/2,V=225+z/2,H=Math.min(0,B),U=Math.max(L,V)-H;f.attr(`viewBox`,`${H} 0 ${U} ${I}`),u(f,I,U,d.useMaxWidth)},`draw`)},styles:N};export{F as diagram};