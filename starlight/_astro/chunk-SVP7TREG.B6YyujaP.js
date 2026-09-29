import{m as e}from"./src.DaJp0K3q.js";import{n as t}from"./chunk-Y2CYZVJY.DsF7k-Jl.js";import{D as n,a as r,b as i,c as a,x as o,z as s}from"./chunk-DU6HZSFF.odZkp4RU.js";import{f as c}from"./mermaid.core.BlI97LaC.js";var l=``,u=``,d=``,f=[],p=new Map,m=t(e=>s(e,o()),`sanitizeText`),h=t(e=>{switch(e.type){case`terminal`:return{...e,value:m(e.value)};case`nonterminal`:return{...e,name:m(e.name)};case`sequence`:return{...e,elements:e.elements.map(h)};case`choice`:return{...e,alternatives:e.alternatives.map(h)};case`optional`:return{...e,element:h(e.element)};case`repetition`:return{...e,element:h(e.element),separator:e.separator?h(e.separator):void 0};case`special`:return{...e,text:m(e.text)}}},`sanitizeAstNode`),g=t(()=>{l=``,u=``,d=``,f.length=0,p.clear(),r(),e.debug(`[Railroad] Database cleared`)},`clear`),_=t(t=>{l=m(t),e.debug(`[Railroad] Title set:`,t)},`setTitle`),v=t(()=>l,`getTitle`),y={clear:g,setTitle:_,getTitle:v,addRule:t(t=>{let n={...t,name:m(t.name),definition:h(t.definition),comment:t.comment?m(t.comment):void 0};e.debug(`[Railroad] Adding rule:`,n.name),p.has(n.name)&&e.warn(`[Railroad] Rule '${n.name}' is already defined. Overwriting.`),f.push(n),p.set(n.name,n)},`addRule`),getRules:t(()=>f,`getRules`),getRule:t(e=>p.get(e),`getRule`),setAccTitle:t(t=>{u=m(t).replace(/^\s+/g,``),e.debug(`[Railroad] Accessibility title set:`,t)},`setAccTitle`),getAccTitle:t(()=>u,`getAccTitle`),setAccDescription:t(t=>{d=m(t).replace(/\n\s+/g,`
`),e.debug(`[Railroad] Accessibility description set:`,t)},`setAccDescription`),getAccDescription:t(()=>d,`getAccDescription`),setDiagramTitle:_,getDiagramTitle:v},b={compactMode:!1,padding:10,verticalSeparation:8,horizontalSeparation:10,arcRadius:10,fontSize:14,fontFamily:`monospace`,terminalFill:`#FFFFC0`,terminalStroke:`#000000`,terminalTextColor:`#000000`,nonTerminalFill:`#FFFFFF`,nonTerminalStroke:`#000000`,nonTerminalTextColor:`#000000`,lineColor:`#000000`,strokeWidth:2,markerFill:`#000000`,commentFill:`#E8E8E8`,commentStroke:`#888888`,commentTextColor:`#666666`,specialFill:`#F0E0FF`,specialStroke:`#8800CC`,ruleNameColor:`#000066`,showMarkers:!0,markerRadius:5},x=/^#(?:[\da-f]{3,4}|[\da-f]{6}|[\da-f]{8})$|^(?:rgb|rgba|hsl|hsla|hwb|lab|lch|oklab|oklch)\([\d\s%+,./-]+\)$|^[a-z]+$/i,S=/^[\w "',.-]+$/,C=new Set([`compactMode`,`padding`,`verticalSeparation`,`horizontalSeparation`,`arcRadius`,`fontSize`,`fontFamily`,`terminalFill`,`terminalStroke`,`terminalTextColor`,`nonTerminalFill`,`nonTerminalStroke`,`nonTerminalTextColor`,`lineColor`,`strokeWidth`,`markerFill`,`commentFill`,`commentStroke`,`commentTextColor`,`specialFill`,`specialStroke`,`ruleNameColor`,`showMarkers`,`markerRadius`]),w=t(e=>e?Object.keys(e).every(e=>e===`railroad`||C.has(e)):!1,`isRailroadStyleOptions`),T=t(e=>e?`railroad`in e&&e.railroad?e.railroad:w(e)?e:{}:{},`extractRailroadOverrides`),E=t(e=>{if(!e||w(e))return{};let{railroad:t,svgId:n,theme:r,look:i,...a}=e;return a},`extractThemeOverrides`),D=t((e,t)=>{if(typeof e!=`string`)return t;let n=e.trim();return x.test(n)?n:t},`sanitizeColorValue`),O=t((e,t)=>{if(typeof e!=`string`)return t;let n=e.trim();return S.test(n)?n:t},`sanitizeFontFamilyValue`),k=t((e,t)=>{let n=typeof e==`number`?e:typeof e==`string`?Number.parseFloat(e):NaN;return Number.isFinite(n)&&n>=0?n:t},`sanitizeNumberValue`),A=t(e=>{let t=typeof e==`number`?e:typeof e==`string`?Number.parseFloat(e):NaN;return Number.isFinite(t)&&t>0?t:void 0},`parseThemeFontSize`),j=t(e=>{let t=O(e.fontFamily,b.fontFamily),n=A(e.fontSize)??b.fontSize;return{...b,fontFamily:t,fontSize:n,terminalFill:D(e.secondBkg??e.secondaryColor,b.terminalFill),terminalStroke:D(e.secondaryBorderColor??e.lineColor,b.terminalStroke),terminalTextColor:D(e.secondaryTextColor??e.textColor,b.terminalTextColor),nonTerminalFill:D(e.mainBkg??e.background,b.nonTerminalFill),nonTerminalStroke:D(e.primaryBorderColor??e.lineColor,b.nonTerminalStroke),nonTerminalTextColor:D(e.primaryTextColor??e.textColor,b.nonTerminalTextColor),lineColor:D(e.lineColor,b.lineColor),markerFill:D(e.lineColor,b.markerFill),commentFill:D(e.labelBackground??e.tertiaryColor,b.commentFill),commentStroke:D(e.tertiaryBorderColor??e.lineColor,b.commentStroke),commentTextColor:D(e.tertiaryTextColor??e.textColor,b.commentTextColor),specialFill:D(e.tertiaryColor??e.secondaryColor,b.specialFill),specialStroke:D(e.tertiaryBorderColor??e.secondaryBorderColor,b.specialStroke),ruleNameColor:D(e.titleColor??e.textColor,b.ruleNameColor)}},`buildThemeDefaults`),M=t(e=>{let t=i(),r=j({...n(),...t.themeVariables??{},...E(e)}),a={...t.railroad??{},...T(e)};return{compactMode:a.compactMode??r.compactMode,padding:k(a.padding,r.padding),verticalSeparation:k(a.verticalSeparation,r.verticalSeparation),horizontalSeparation:k(a.horizontalSeparation,r.horizontalSeparation),arcRadius:k(a.arcRadius,r.arcRadius),fontSize:k(a.fontSize,r.fontSize),fontFamily:O(a.fontFamily,r.fontFamily),terminalFill:D(a.terminalFill,r.terminalFill),terminalStroke:D(a.terminalStroke,r.terminalStroke),terminalTextColor:D(a.terminalTextColor,r.terminalTextColor),nonTerminalFill:D(a.nonTerminalFill,r.nonTerminalFill),nonTerminalStroke:D(a.nonTerminalStroke,r.nonTerminalStroke),nonTerminalTextColor:D(a.nonTerminalTextColor,r.nonTerminalTextColor),lineColor:D(a.lineColor,r.lineColor),strokeWidth:k(a.strokeWidth,r.strokeWidth),markerFill:D(a.markerFill,r.markerFill),commentFill:D(a.commentFill,r.commentFill),commentStroke:D(a.commentStroke,r.commentStroke),commentTextColor:D(a.commentTextColor,r.commentTextColor),specialFill:D(a.specialFill,r.specialFill),specialStroke:D(a.specialStroke,r.specialStroke),ruleNameColor:D(a.ruleNameColor,r.ruleNameColor),showMarkers:a.showMarkers??r.showMarkers,markerRadius:k(a.markerRadius,r.markerRadius)}},`buildRailroadStyleOptions`),N=t(e=>{let{fontFamily:t,fontSize:n,terminalFill:r,terminalStroke:i,terminalTextColor:a,nonTerminalFill:o,nonTerminalStroke:s,nonTerminalTextColor:c,lineColor:l,strokeWidth:u,markerFill:d,commentFill:f,commentStroke:p,commentTextColor:m,specialFill:h,specialStroke:g,ruleNameColor:_}=M(e);return`
  .railroad-diagram {
    font-family: ${t};
    font-size: ${n}px;
  }

  .railroad-terminal rect {
    fill: ${r};
    stroke: ${i};
    stroke-width: ${u}px;
  }

  .railroad-terminal text {
    fill: ${a};
    font-family: ${t};
    font-size: ${n}px;
    text-anchor: middle;
    dominant-baseline: middle;
  }

  .railroad-nonterminal rect {
    fill: ${o};
    stroke: ${s};
    stroke-width: ${u}px;
  }

  .railroad-nonterminal text {
    fill: ${c};
    font-family: ${t};
    font-size: ${n}px;
    text-anchor: middle;
    dominant-baseline: middle;
  }

  .railroad-line {
    stroke: ${l};
    stroke-width: ${u}px;
    fill: none;
  }

  .railroad-start circle,
  .railroad-end circle {
    fill: ${d};
  }

  .railroad-comment ellipse {
    fill: ${f};
    stroke: ${p};
    stroke-width: ${u}px;
  }

  .railroad-comment text {
    fill: ${m};
    font-style: italic;
    font-family: ${t};
    font-size: ${n}px;
    text-anchor: middle;
    dominant-baseline: middle;
  }

  .railroad-special rect {
    fill: ${h};
    stroke: ${g};
    stroke-width: ${u}px;
    stroke-dasharray: 5,3;
  }

  .railroad-special text {
    fill: ${c};
    font-family: ${t};
    font-size: ${n}px;
    text-anchor: middle;
    dominant-baseline: middle;
  }

  .railroad-rule-name {
    font-weight: bold;
    fill: ${_};
    font-family: ${t};
    font-size: ${n}px;
  }

  .railroad-group {
    /* Grouping container, no specific styles */
  }
`},`getStyles`),P=class{constructor(){this.d=``}static{t(this,`PathBuilder`)}moveTo(e,t){return this.d+=`M ${e} ${t} `,this}lineTo(e,t){return this.d+=`L ${e} ${t} `,this}horizontalTo(e){return this.d+=`H ${e} `,this}verticalTo(e){return this.d+=`V ${e} `,this}arcTo(e,t,n,r,i,a,o){return this.d+=`A ${e} ${t} ${n} ${+!!r} ${+!!i} ${a} ${o} `,this}build(){return this.d.trim()}},F=class{constructor(e,t=M()){this.textCache=new Map,this.svg=e,this.config=t}static{t(this,`RailroadRenderer`)}measureText(e){if(this.textCache.has(e))return this.textCache.get(e);let t=this.svg.append(`text`).attr(`font-family`,this.config.fontFamily).attr(`font-size`,this.config.fontSize).text(e),n=t.node().getBBox(),r={width:n.width,height:n.height};return t.remove(),this.textCache.set(e,r),r}renderTerminal(e,t){let n=this.measureText(t),r=n.width+this.config.padding*2,i=n.height+this.config.padding*2,a=e.append(`g`).attr(`class`,`railroad-terminal`);return a.append(`rect`).attr(`x`,0).attr(`y`,0).attr(`width`,r).attr(`height`,i).attr(`rx`,10).attr(`ry`,10),a.append(`text`).attr(`x`,r/2).attr(`y`,i/2).text(t),{element:a.node(),dimensions:{width:r,height:i,up:i/2,down:i/2}}}renderNonTerminal(e,t){let n=this.measureText(t),r=n.width+this.config.padding*2,i=n.height+this.config.padding*2,a=e.append(`g`).attr(`class`,`railroad-nonterminal`);return a.append(`rect`).attr(`x`,0).attr(`y`,0).attr(`width`,r).attr(`height`,i),a.append(`text`).attr(`x`,r/2).attr(`y`,i/2).text(t),{element:a.node(),dimensions:{width:r,height:i,up:i/2,down:i/2}}}renderSequence(e,t){let n=t.map(t=>this.renderExpression(e,t)),r=0,i=0,a=0;for(let e of n)r+=e.dimensions.width,i=Math.max(i,e.dimensions.up),a=Math.max(a,e.dimensions.down);r+=(n.length-1)*this.config.horizontalSeparation;let o=e.append(`g`).attr(`class`,`railroad-sequence`),s=0;for(let e=0;e<n.length;e++){let t=n[e],r=i-t.dimensions.up;if(o.node().appendChild(t.element).setAttribute(`transform`,`translate(${s}, ${r})`),e<n.length-1){let e=s+t.dimensions.width,n=e+this.config.horizontalSeparation,r=i;o.append(`path`).attr(`class`,`railroad-line`).attr(`d`,new P().moveTo(e,r).lineTo(n,r).build())}s+=t.dimensions.width+this.config.horizontalSeparation}return{element:o.node(),dimensions:{width:r,height:i+a,up:i,down:a}}}renderChoice(e,t){let n=t.map(t=>this.renderExpression(e,t)),r=0,i=0;for(let e of n)r=Math.max(r,e.dimensions.width),i+=e.dimensions.height;i+=(n.length-1)*this.config.verticalSeparation;let a=this.config.arcRadius,o=a*4,s=r+o,c=e.append(`g`).attr(`class`,`railroad-choice`),l=0,u=i/2;for(let e of n){let t=l,n=t+e.dimensions.up,i=a*2+(r-e.dimensions.width)/2;c.node().appendChild(e.element).setAttribute(`transform`,`translate(${i}, ${t})`);let o=new P,d=n>u;n===u?o.moveTo(0,u).lineTo(i,n):o.moveTo(0,u).arcTo(a,a,0,!1,d,a,u+(d?a:-a)).lineTo(a,n-(d?a:-a)).arcTo(a,a,0,!1,!d,a*2,n).lineTo(i,n),c.append(`path`).attr(`class`,`railroad-line`).attr(`d`,o.build());let f=new P,p=i+e.dimensions.width,m=s-a*2;n===u?f.moveTo(p,n).lineTo(s,u):f.moveTo(p,n).lineTo(m,n).arcTo(a,a,0,!1,!d,s-a,n+(d?-a:a)).lineTo(s-a,u+(d?a:-a)).arcTo(a,a,0,!1,d,s,u),c.append(`path`).attr(`class`,`railroad-line`).attr(`d`,f.build()),l+=e.dimensions.height+this.config.verticalSeparation}return{element:c.node(),dimensions:{width:s,height:i,up:u,down:i-u}}}renderOptional(e,t){let n=this.renderExpression(e,t),r=this.config.arcRadius,i=r*2,a=n.dimensions.width+r*4,o=n.dimensions.height+i,s=e.append(`g`).attr(`class`,`railroad-optional`),c=r*2,l=i;s.node().appendChild(n.element).setAttribute(`transform`,`translate(${c}, ${l})`);let u=l+n.dimensions.up,d=new P().moveTo(0,u).lineTo(r*2,u);s.append(`path`).attr(`class`,`railroad-line`).attr(`d`,d.build());let f=new P().moveTo(c+n.dimensions.width,u).lineTo(a,u);s.append(`path`).attr(`class`,`railroad-line`).attr(`d`,f.build());let p=new P().moveTo(0,u).arcTo(r,r,0,!1,!1,r,u-r).lineTo(r,r).arcTo(r,r,0,!1,!0,r*2,0).lineTo(a-r*2,0).arcTo(r,r,0,!1,!0,a-r,r).lineTo(a-r,u-r).arcTo(r,r,0,!1,!1,a,u);return s.append(`path`).attr(`class`,`railroad-line`).attr(`d`,p.build()),{element:s.node(),dimensions:{width:a,height:o,up:u,down:o-u}}}renderRepetition(e,t,n){let r=this.renderExpression(e,t),i=this.config.arcRadius,a=i*2,o=r.dimensions.width+i*4,s=n===0,c=r.dimensions.height+a+(s?a:0),l=e.append(`g`).attr(`class`,`railroad-repetition`),u=i*2,d=s?a:0;l.node().appendChild(r.element).setAttribute(`transform`,`translate(${u}, ${d})`);let f=d+r.dimensions.up;l.append(`path`).attr(`class`,`railroad-line`).attr(`d`,new P().moveTo(0,f).lineTo(i*2,f).build()),l.append(`path`).attr(`class`,`railroad-line`).attr(`d`,new P().moveTo(u+r.dimensions.width,f).lineTo(o,f).build());let p=d+r.dimensions.height+i,m=new P().moveTo(u+r.dimensions.width,f).arcTo(i,i,0,!1,!0,u+r.dimensions.width+i,f+i).lineTo(u+r.dimensions.width+i,p).arcTo(i,i,0,!1,!0,u+r.dimensions.width,p+i).lineTo(i*2,p+i).arcTo(i,i,0,!1,!0,i,p).lineTo(i,f+i).arcTo(i,i,0,!1,!0,i*2,f);if(l.append(`path`).attr(`class`,`railroad-line`).attr(`d`,m.build()),s){let e=new P().moveTo(0,f).arcTo(i,i,0,!1,!1,i,f-i).lineTo(i,i).arcTo(i,i,0,!1,!0,i*2,0).lineTo(o-i*2,0).arcTo(i,i,0,!1,!0,o-i,i).lineTo(o-i,f-i).arcTo(i,i,0,!1,!1,o,f);l.append(`path`).attr(`class`,`railroad-line`).attr(`d`,e.build())}return{element:l.node(),dimensions:{width:o,height:c,up:f,down:c-f}}}renderSpecial(e,t){let n=this.measureText(`? `+t+` ?`),r=n.width+this.config.padding*2,i=n.height+this.config.padding*2,a=e.append(`g`).attr(`class`,`railroad-special`);return a.append(`rect`).attr(`x`,0).attr(`y`,0).attr(`width`,r).attr(`height`,i),a.append(`text`).attr(`x`,r/2).attr(`y`,i/2).text(`? `+t+` ?`),{element:a.node(),dimensions:{width:r,height:i,up:i/2,down:i/2}}}renderExpression(e,t){switch(t.type){case`terminal`:return this.renderTerminal(e,t.value);case`nonterminal`:return this.renderNonTerminal(e,t.name);case`sequence`:return this.renderSequence(e,t.elements);case`choice`:return this.renderChoice(e,t.alternatives);case`optional`:return this.renderOptional(e,t.element);case`repetition`:return this.renderRepetition(e,t.element,t.min);case`special`:return this.renderSpecial(e,t.text);default:throw Error(`Unknown node type: ${t.type}`)}}renderRule(e,t){let n=this.svg.append(`g`).attr(`class`,`railroad-rule`).attr(`transform`,`translate(0, ${t})`),r=e.name+` =`,i=this.measureText(r).width+20,a=i+20,o=n.append(`g`),s=this.renderExpression(o,e.definition),c=Math.max(20,s.dimensions.up),l=c-s.dimensions.up;return o.attr(`transform`,`translate(${a}, ${l})`),n.append(`g`).attr(`class`,`railroad-rule-name-group`).append(`text`).attr(`class`,`railroad-rule-name`).attr(`x`,0).attr(`y`,c).text(r),n.append(`g`).attr(`class`,`railroad-start`).append(`circle`).attr(`cx`,i).attr(`cy`,c).attr(`r`,this.config.markerRadius),n.append(`g`).attr(`class`,`railroad-end`).append(`circle`).attr(`cx`,a+s.dimensions.width+10).attr(`cy`,c).attr(`r`,this.config.markerRadius),n.append(`path`).attr(`class`,`railroad-line`).attr(`d`,new P().moveTo(i+this.config.markerRadius,c).lineTo(a,c).build()),n.append(`path`).attr(`class`,`railroad-line`).attr(`d`,new P().moveTo(a+s.dimensions.width,c).lineTo(a+s.dimensions.width+10-this.config.markerRadius,c).build()),{height:Math.max(40,l+s.dimensions.height+this.config.padding*2),width:a+s.dimensions.width+10+this.config.markerRadius}}renderDiagram(e){let t=this.config.padding,n=0;for(let r of e){let e=this.renderRule(r,t);t+=e.height+this.config.verticalSeparation,n=Math.max(n,e.width)}return{width:n+this.config.padding*2,height:t+this.config.padding}}},I=t((e,t,n)=>{a(e,t.height,t.width,n),e.attr(`viewBox`,`0 0 ${t.width} ${t.height}`)},`configureRailroadSvgSize`),L={draw:t((t,n,r)=>{e.debug(`[Railroad] Rendering diagram
`+t);try{let t=c(n);t.attr(`class`,`railroad-diagram`);let r=i().railroad?.useMaxWidth??!0,a=y.getRules();if(e.debug(`[Railroad] Rendering ${a.length} rules`),a.length===0){e.warn(`[Railroad] No rules to render`),I(t,{height:100,width:200},r);return}I(t,new F(t,M()).renderDiagram(a),r),e.debug(`[Railroad] Render complete`)}catch(t){throw e.error(`[Railroad] Render error:`,t),t}},`draw`)};export{N as n,L as r,y as t};