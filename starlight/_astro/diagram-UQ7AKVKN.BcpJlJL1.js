import{m as e}from"./src.DaJp0K3q.js";import{n as t}from"./mermaid-parser.core.DX-EkrVF.js";import{n}from"./chunk-Y2CYZVJY.DsF7k-Jl.js";import{D as r,H as i,K as a,U as o,a as s,b as c,c as l,f as u,v as d,w as f,y as p}from"./chunk-DU6HZSFF.odZkp4RU.js";import{i as m}from"./chunk-75Z2AOVW.Bt3ztROe.js";import{t as h}from"./chunk-JWPE2WC7.A3RzWITa.js";import{f as g}from"./mermaid.core.BlI97LaC.js";var _={showLegend:!0,ticks:5,max:null,min:0,graticule:`circle`},v=32,y={axes:[],curves:[],options:_},b=structuredClone(y),x=u.radar,S=n(()=>m({...x,...c().radar}),`getConfig`),C=n(()=>b.axes,`getAxes`),w=n(()=>b.curves,`getCurves`),T=n(()=>b.options,`getOptions`),E=n(e=>{b.axes=e.map(e=>({name:e.name,label:e.label??e.name}))},`setAxes`),D=n(e=>{b.curves=e.map(e=>({name:e.name,label:e.label??e.name,entries:O(e.entries)}))},`setCurves`),O=n(e=>{if(e[0].axis==null)return e.map(e=>e.value);let t=C();if(t.length===0)throw Error(`Axes must be populated before curves for reference entries`);return t.map(t=>{let n=e.find(e=>e.axis?.$refText===t.name);if(n===void 0)throw Error(`Missing entry for axis `+t.label);return n.value})},`computeCurveEntries`),k={getAxes:C,getCurves:w,getOptions:T,setAxes:E,setCurves:D,setOptions:n(t=>{let n=t.reduce((e,t)=>(e[t.name]=t,e),{});b.options={showLegend:n.showLegend?.value??_.showLegend,ticks:n.ticks?.value??_.ticks,max:n.max?.value??_.max,min:n.min?.value??_.min,graticule:n.graticule?.value??_.graticule},b.options.ticks>v&&(e.warn(`Radar diagram ticks (${b.options.ticks}) exceeds maximum allowed (${v}). Using ${v} instead.`),b.options.ticks=v)},`setOptions`),getConfig:S,clear:n(()=>{s(),b=structuredClone(y)},`clear`),setAccTitle:o,getAccTitle:p,setDiagramTitle:a,getDiagramTitle:f,getAccDescription:d,setAccDescription:i},A=n(e=>{h(e,k);let{axes:t,curves:n,options:r}=e;k.setAxes(t),k.setCurves(n),k.setOptions(r)},`populate`),j={parse:n(async n=>{let r=await t(`radar`,n);e.debug(r),A(r)},`parse`)},M=n((e,t,n,r)=>{let i=r.db,a=i.getAxes(),o=i.getCurves(),s=i.getOptions(),c=i.getConfig(),l=i.getDiagramTitle(),u=N(g(t),c),d=s.max??Math.max(...o.map(e=>Math.max(...e.entries))),f=s.min,p=Math.min(c.width,c.height)/2;P(u,a,p,s.ticks,s.graticule),F(u,a,p,c),I(u,a,o,f,d,s.graticule,c),z(u,o,s.showLegend,c),u.append(`text`).attr(`class`,`radarTitle`).text(l).attr(`x`,0).attr(`y`,-c.height/2-c.marginTop)},`draw`),N=n((e,t)=>{let n=t.width+t.marginLeft+t.marginRight,r=t.height+t.marginTop+t.marginBottom,i={x:t.marginLeft+t.width/2,y:t.marginTop+t.height/2};return l(e,r,n,t.useMaxWidth??!0),e.attr(`viewBox`,`0 0 ${n} ${r}`).attr(`overflow`,`visible`),e.append(`g`).attr(`transform`,`translate(${i.x}, ${i.y})`)},`drawFrame`),P=n((e,t,n,r,i)=>{if(i===`circle`)for(let t=0;t<r;t++){let i=n*(t+1)/r;e.append(`circle`).attr(`r`,i).attr(`class`,`radarGraticule`)}else if(i===`polygon`){let i=t.length;for(let a=0;a<r;a++){let o=n*(a+1)/r,s=t.map((e,t)=>{let n=2*t*Math.PI/i-Math.PI/2;return`${o*Math.cos(n)},${o*Math.sin(n)}`}).join(` `);e.append(`polygon`).attr(`points`,s).attr(`class`,`radarGraticule`)}}},`drawGraticule`),F=n((e,t,n,r)=>{let i=t.length;for(let a=0;a<i;a++){let o=t[a].label,s=2*a*Math.PI/i-Math.PI/2,c=Math.cos(s),l=Math.sin(s);e.append(`line`).attr(`x1`,0).attr(`y1`,0).attr(`x2`,n*r.axisScaleFactor*c).attr(`y2`,n*r.axisScaleFactor*l).attr(`class`,`radarAxisLine`);let u=c>.01?`start`:c<-.01?`end`:`middle`,d=l>.01?`hanging`:l<-.01?`auto`:`central`;e.append(`text`).text(o).attr(`x`,n*r.axisLabelFactor*c+4*c).attr(`y`,n*r.axisLabelFactor*l+4*l).attr(`text-anchor`,u).attr(`dominant-baseline`,d).attr(`class`,`radarAxisLabel`)}},`drawAxes`);function I(e,t,n,r,i,a,o){let s=t.length,c=Math.min(o.width,o.height)/2;n.forEach((t,n)=>{if(t.entries.length!==s)return;let l=t.entries.map((e,t)=>{let n=2*Math.PI*t/s-Math.PI/2,a=L(e,r,i,c);return{x:a*Math.cos(n),y:a*Math.sin(n)}});a===`circle`?e.append(`path`).attr(`d`,R(l,o.curveTension)).attr(`class`,`radarCurve-${n}`):a===`polygon`&&e.append(`polygon`).attr(`points`,l.map(e=>`${e.x},${e.y}`).join(` `)).attr(`class`,`radarCurve-${n}`)})}n(I,`drawCurves`);function L(e,t,n,r){return r*(Math.min(Math.max(e,t),n)-t)/(n-t)}n(L,`relativeRadius`);function R(e,t){let n=e.length,r=`M${e[0].x},${e[0].y}`;for(let i=0;i<n;i++){let a=e[(i-1+n)%n],o=e[i],s=e[(i+1)%n],c=e[(i+2)%n],l={x:o.x+(s.x-a.x)*t,y:o.y+(s.y-a.y)*t},u={x:s.x-(c.x-o.x)*t,y:s.y-(c.y-o.y)*t};r+=` C${l.x},${l.y} ${u.x},${u.y} ${s.x},${s.y}`}return`${r} Z`}n(R,`closedRoundCurve`);function z(e,t,n,r){if(!n)return;let i=(r.width/2+r.marginRight)*3/4,a=-(r.height/2+r.marginTop)*3/4;t.forEach((t,n)=>{let r=e.append(`g`).attr(`transform`,`translate(${i}, ${a+n*20})`);r.append(`rect`).attr(`width`,12).attr(`height`,12).attr(`class`,`radarLegendBox-${n}`),r.append(`text`).attr(`x`,16).attr(`y`,0).attr(`class`,`radarLegendText`).text(t.label)})}n(z,`drawLegend`);var B={draw:M},V=n((e,t)=>{let n=``;for(let r=0;r<e.THEME_COLOR_LIMIT;r++){let i=e[`cScale${r}`];n+=`
		.radarCurve-${r} {
			color: ${i};
			fill: ${i};
			fill-opacity: ${t.curveOpacity};
			stroke: ${i};
			stroke-width: ${t.curveStrokeWidth};
		}
		.radarLegendBox-${r} {
			fill: ${i};
			fill-opacity: ${t.curveOpacity};
			stroke: ${i};
		}
		`}return n},`genIndexStyles`),H=n(e=>{let t=r(),n=c(),i=m(t,n.themeVariables);return{themeVariables:i,radarOptions:m(i.radar,e)}},`buildRadarStyleOptions`),U={parser:j,db:k,renderer:B,styles:n(({radar:e}={})=>{let{themeVariables:t,radarOptions:n}=H(e);return`
	.radarTitle {
		font-size: ${t.fontSize};
		color: ${t.titleColor};
		dominant-baseline: hanging;
		text-anchor: middle;
	}
	.radarAxisLine {
		stroke: ${n.axisColor};
		stroke-width: ${n.axisStrokeWidth};
	}
	.radarAxisLabel {
		font-size: ${n.axisLabelFontSize}px;
		color: ${n.axisColor};
	}
	.radarGraticule {
		fill: ${n.graticuleColor};
		fill-opacity: ${n.graticuleOpacity};
		stroke: ${n.graticuleColor};
		stroke-width: ${n.graticuleStrokeWidth};
	}
	.radarLegendText {
		text-anchor: start;
		font-size: ${n.legendFontSize}px;
		dominant-baseline: hanging;
	}
	${V(t,n)}
	`},`styles`)};export{U as diagram};