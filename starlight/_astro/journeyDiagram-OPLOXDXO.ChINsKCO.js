import{t as e}from"./chunk-5VM5RSS4.TOl2eSYi.js";import{a as t,n,o as r,s as i}from"./chunk-HFQBQIA3.7oKsMhA6.js";import{C as a,H as o,U as s,_ as c,a as l,b as u,c as d,q as f,v as p}from"./chunk-BG4553SI.DzMjcvML.js";import{t as m}from"./arc.cxkLdl1s.js";import{p as h}from"./src.Duoub3vK.js";import{n as g}from"./chunk-Y2CYZVJY.DsF7k-Jl.js";var _=(function(){var e=g(function(e,t,n,r){for(n||={},r=e.length;r--;n[e[r]]=t);return n},`o`),t=[6,8,10,11,12,14,16,17,18],n=[1,9],r=[1,10],i=[1,11],a=[1,12],o=[1,13],s=[1,14],c={trace:g(function(){},`trace`),yy:{},symbols_:{error:2,start:3,journey:4,document:5,EOF:6,line:7,SPACE:8,statement:9,NEWLINE:10,title:11,acc_title:12,acc_title_value:13,acc_descr:14,acc_descr_value:15,acc_descr_multiline_value:16,section:17,taskName:18,taskData:19,$accept:0,$end:1},terminals_:{2:`error`,4:`journey`,6:`EOF`,8:`SPACE`,10:`NEWLINE`,11:`title`,12:`acc_title`,13:`acc_title_value`,14:`acc_descr`,15:`acc_descr_value`,16:`acc_descr_multiline_value`,17:`section`,18:`taskName`,19:`taskData`},productions_:[0,[3,3],[5,0],[5,2],[7,2],[7,1],[7,1],[7,1],[9,1],[9,2],[9,2],[9,1],[9,1],[9,2]],performAction:g(function(e,t,n,r,i,a,o){var s=a.length-1;switch(i){case 1:return a[s-1];case 2:this.$=[];break;case 3:a[s-1].push(a[s]),this.$=a[s-1];break;case 4:case 5:this.$=a[s];break;case 6:case 7:this.$=[];break;case 8:r.setDiagramTitle(a[s].substr(6)),this.$=a[s].substr(6);break;case 9:this.$=a[s].trim(),r.setAccTitle(this.$);break;case 10:case 11:this.$=a[s].trim(),r.setAccDescription(this.$);break;case 12:r.addSection(a[s].substr(8)),this.$=a[s].substr(8);break;case 13:r.addTask(a[s-1],a[s]),this.$=`task`}},`anonymous`),table:[{3:1,4:[1,2]},{1:[3]},e(t,[2,2],{5:3}),{6:[1,4],7:5,8:[1,6],9:7,10:[1,8],11:n,12:r,14:i,16:a,17:o,18:s},e(t,[2,7],{1:[2,1]}),e(t,[2,3]),{9:15,11:n,12:r,14:i,16:a,17:o,18:s},e(t,[2,5]),e(t,[2,6]),e(t,[2,8]),{13:[1,16]},{15:[1,17]},e(t,[2,11]),e(t,[2,12]),{19:[1,18]},e(t,[2,4]),e(t,[2,9]),e(t,[2,10]),e(t,[2,13])],defaultActions:{},parseError:g(function(e,t){if(t.recoverable)this.trace(e);else{var n=Error(e);throw n.hash=t,n}},`parseError`),parse:g(function(e){var t=this,n=[0],r=[],i=[null],a=[],o=this.table,s=``,c=0,l=0,u=0,d=2,f=1,p=a.slice.call(arguments,1),m=Object.create(this.lexer),h={yy:{}};for(var _ in this.yy)Object.prototype.hasOwnProperty.call(this.yy,_)&&(h.yy[_]=this.yy[_]);m.setInput(e,h.yy),h.yy.lexer=m,h.yy.parser=this,m.yylloc===void 0&&(m.yylloc={});var v=m.yylloc;a.push(v);var y=m.options&&m.options.ranges;this.parseError=typeof h.yy.parseError==`function`?h.yy.parseError:Object.getPrototypeOf(this).parseError;function b(e){n.length-=2*e,i.length-=e,a.length-=e}g(b,`popStack`);function x(){var e=r.pop()||m.lex()||f;return typeof e!=`number`&&(e instanceof Array&&(r=e,e=r.pop()),e=t.symbols_[e]||e),e}g(x,`lex`);for(var S,C,w,T,E,D={},O,k,A,j;;){if(w=n[n.length-1],this.defaultActions[w]?T=this.defaultActions[w]:(S??=x(),T=o[w]&&o[w][S]),T===void 0||!T.length||!T[0]){var M=``;for(O in j=[],o[w])this.terminals_[O]&&O>d&&j.push(`'`+this.terminals_[O]+`'`);M=m.showPosition?`Parse error on line `+(c+1)+`:
`+m.showPosition()+`
Expecting `+j.join(`, `)+`, got '`+(this.terminals_[S]||S)+`'`:`Parse error on line `+(c+1)+`: Unexpected `+(S==f?`end of input`:`'`+(this.terminals_[S]||S)+`'`),this.parseError(M,{text:m.match,token:this.terminals_[S]||S,line:m.yylineno,loc:v,expected:j})}if(T[0]instanceof Array&&T.length>1)throw Error(`Parse Error: multiple actions possible at state: `+w+`, token: `+S);switch(T[0]){case 1:n.push(S),i.push(m.yytext),a.push(m.yylloc),n.push(T[1]),S=null,C?(S=C,C=null):(l=m.yyleng,s=m.yytext,c=m.yylineno,v=m.yylloc,u>0&&u--);break;case 2:if(k=this.productions_[T[1]][1],D.$=i[i.length-k],D._$={first_line:a[a.length-(k||1)].first_line,last_line:a[a.length-1].last_line,first_column:a[a.length-(k||1)].first_column,last_column:a[a.length-1].last_column},y&&(D._$.range=[a[a.length-(k||1)].range[0],a[a.length-1].range[1]]),E=this.performAction.apply(D,[s,l,c,h.yy,T[1],i,a].concat(p)),E!==void 0)return E;k&&(n=n.slice(0,-1*k*2),i=i.slice(0,-1*k),a=a.slice(0,-1*k)),n.push(this.productions_[T[1]][0]),i.push(D.$),a.push(D._$),A=o[n[n.length-2]][n[n.length-1]],n.push(A);break;case 3:return!0}}return!0},`parse`)};c.lexer=(function(){return{EOF:1,parseError:g(function(e,t){if(this.yy.parser)this.yy.parser.parseError(e,t);else throw Error(e)},`parseError`),setInput:g(function(e,t){return this.yy=t||this.yy||{},this._input=e,this._more=this._backtrack=this.done=!1,this.yylineno=this.yyleng=0,this.yytext=this.matched=this.match=``,this.conditionStack=[`INITIAL`],this.yylloc={first_line:1,first_column:0,last_line:1,last_column:0},this.options.ranges&&(this.yylloc.range=[0,0]),this.offset=0,this},`setInput`),input:g(function(){var e=this._input[0];return this.yytext+=e,this.yyleng++,this.offset++,this.match+=e,this.matched+=e,e.match(/(?:\r\n?|\n).*/g)?(this.yylineno++,this.yylloc.last_line++):this.yylloc.last_column++,this.options.ranges&&this.yylloc.range[1]++,this._input=this._input.slice(1),e},`input`),unput:g(function(e){var t=e.length,n=e.split(/(?:\r\n?|\n)/g);this._input=e+this._input,this.yytext=this.yytext.substr(0,this.yytext.length-t),this.offset-=t;var r=this.match.split(/(?:\r\n?|\n)/g);this.match=this.match.substr(0,this.match.length-1),this.matched=this.matched.substr(0,this.matched.length-1),n.length-1&&(this.yylineno-=n.length-1);var i=this.yylloc.range;return this.yylloc={first_line:this.yylloc.first_line,last_line:this.yylineno+1,first_column:this.yylloc.first_column,last_column:n?(n.length===r.length?this.yylloc.first_column:0)+r[r.length-n.length].length-n[0].length:this.yylloc.first_column-t},this.options.ranges&&(this.yylloc.range=[i[0],i[0]+this.yyleng-t]),this.yyleng=this.yytext.length,this},`unput`),more:g(function(){return this._more=!0,this},`more`),reject:g(function(){if(this.options.backtrack_lexer)this._backtrack=!0;else return this.parseError(`Lexical error on line `+(this.yylineno+1)+`. You can only invoke reject() in the lexer when the lexer is of the backtracking persuasion (options.backtrack_lexer = true).
`+this.showPosition(),{text:``,token:null,line:this.yylineno});return this},`reject`),less:g(function(e){this.unput(this.match.slice(e))},`less`),pastInput:g(function(){var e=this.matched.substr(0,this.matched.length-this.match.length);return(e.length>20?`...`:``)+e.substr(-20).replace(/\n/g,``)},`pastInput`),upcomingInput:g(function(){var e=this.match;return e.length<20&&(e+=this._input.substr(0,20-e.length)),(e.substr(0,20)+(e.length>20?`...`:``)).replace(/\n/g,``)},`upcomingInput`),showPosition:g(function(){var e=this.pastInput(),t=Array(e.length+1).join(`-`);return e+this.upcomingInput()+`
`+t+`^`},`showPosition`),test_match:g(function(e,t){var n,r,i;if(this.options.backtrack_lexer&&(i={yylineno:this.yylineno,yylloc:{first_line:this.yylloc.first_line,last_line:this.last_line,first_column:this.yylloc.first_column,last_column:this.yylloc.last_column},yytext:this.yytext,match:this.match,matches:this.matches,matched:this.matched,yyleng:this.yyleng,offset:this.offset,_more:this._more,_input:this._input,yy:this.yy,conditionStack:this.conditionStack.slice(0),done:this.done},this.options.ranges&&(i.yylloc.range=this.yylloc.range.slice(0))),r=e[0].match(/(?:\r\n?|\n).*/g),r&&(this.yylineno+=r.length),this.yylloc={first_line:this.yylloc.last_line,last_line:this.yylineno+1,first_column:this.yylloc.last_column,last_column:r?r[r.length-1].length-r[r.length-1].match(/\r?\n?/)[0].length:this.yylloc.last_column+e[0].length},this.yytext+=e[0],this.match+=e[0],this.matches=e,this.yyleng=this.yytext.length,this.options.ranges&&(this.yylloc.range=[this.offset,this.offset+=this.yyleng]),this._more=!1,this._backtrack=!1,this._input=this._input.slice(e[0].length),this.matched+=e[0],n=this.performAction.call(this,this.yy,this,t,this.conditionStack[this.conditionStack.length-1]),this.done&&this._input&&(this.done=!1),n)return n;if(this._backtrack){for(var a in i)this[a]=i[a];return!1}return!1},`test_match`),next:g(function(){if(this.done)return this.EOF;this._input||(this.done=!0);var e,t,n,r;this._more||(this.yytext=``,this.match=``);for(var i=this._currentRules(),a=0;a<i.length;a++)if(n=this._input.match(this.rules[i[a]]),n&&(!t||n[0].length>t[0].length)){if(t=n,r=a,this.options.backtrack_lexer){if(e=this.test_match(n,i[a]),e!==!1)return e;if(this._backtrack){t=!1;continue}return!1}if(!this.options.flex)break}return t?(e=this.test_match(t,i[r]),e!==!1&&e):this._input===``?this.EOF:this.parseError(`Lexical error on line `+(this.yylineno+1)+`. Unrecognized text.
`+this.showPosition(),{text:``,token:null,line:this.yylineno})},`next`),lex:g(function(){return this.next()||this.lex()},`lex`),begin:g(function(e){this.conditionStack.push(e)},`begin`),popState:g(function(){return this.conditionStack.length-1>0?this.conditionStack.pop():this.conditionStack[0]},`popState`),_currentRules:g(function(){return this.conditionStack.length&&this.conditionStack[this.conditionStack.length-1]?this.conditions[this.conditionStack[this.conditionStack.length-1]].rules:this.conditions.INITIAL.rules},`_currentRules`),topState:g(function(e){return e=this.conditionStack.length-1-Math.abs(e||0),e>=0?this.conditionStack[e]:`INITIAL`},`topState`),pushState:g(function(e){this.begin(e)},`pushState`),stateStackSize:g(function(){return this.conditionStack.length},`stateStackSize`),options:{"case-insensitive":!0},performAction:g(function(e,t,n,r){switch(n){case 0:break;case 1:break;case 2:return 10;case 3:break;case 4:break;case 5:return 4;case 6:return 11;case 7:return this.begin(`acc_title`),12;case 8:return this.popState(),`acc_title_value`;case 9:return this.begin(`acc_descr`),14;case 10:return this.popState(),`acc_descr_value`;case 11:this.begin(`acc_descr_multiline`);break;case 12:this.popState();break;case 13:return`acc_descr_multiline_value`;case 14:return 17;case 15:return 18;case 16:return 19;case 17:return`:`;case 18:return 6;case 19:return`INVALID`}},`anonymous`),rules:[/^(?:%(?!\{)[^\n]*)/i,/^(?:[^\}]%%[^\n]*)/i,/^(?:[\n]+)/i,/^(?:\s+)/i,/^(?:#[^\n]*)/i,/^(?:journey\b)/i,/^(?:title\s[^#\n;]+)/i,/^(?:accTitle\s*:\s*)/i,/^(?:(?!\n||)*[^\n]*)/i,/^(?:accDescr\s*:\s*)/i,/^(?:(?!\n||)*[^\n]*)/i,/^(?:accDescr\s*\{\s*)/i,/^(?:[\}])/i,/^(?:[^\}]*)/i,/^(?:section\s[^#:\n;]+)/i,/^(?:[^#:\n;]+)/i,/^(?::[^#\n;]+)/i,/^(?::)/i,/^(?:$)/i,/^(?:.)/i],conditions:{acc_descr_multiline:{rules:[12,13],inclusive:!1},acc_descr:{rules:[10],inclusive:!1},acc_title:{rules:[8],inclusive:!1},INITIAL:{rules:[0,1,2,3,4,5,6,7,9,11,14,15,16,17,18,19],inclusive:!0}}}})();function l(){this.yy={}}return g(l,`Parser`),l.prototype=c,c.Parser=l,new l})();_.parser=_;var v=_,y=``,b=[],x=[],S=[],C=g(function(){b.length=0,x.length=0,y=``,S.length=0,l()},`clear`),w=g(function(e){y=e,b.push(e)},`addSection`),T=g(function(){return b},`getSections`),E=g(function(){let e=A(),t=0;for(;!e&&t<100;)e=A(),t++;return x.push(...S),x},`getTasks`),D=g(function(){let e=[];return x.forEach(t=>{t.people&&e.push(...t.people)}),[...new Set(e)].sort()},`updateActors`),O=g(function(e,t){let n=t.substr(1).split(`:`),r=0,i=[];n.length===1?(r=Number(n[0]),i=[]):(r=Number(n[0]),i=n[1].split(`,`));let a=i.map(e=>e.trim()),o={section:y,type:y,people:a,task:e,score:r};S.push(o)},`addTask`),k=g(function(e){let t={section:y,type:y,description:e,task:e,classes:[]};x.push(t)},`addTaskOrg`),A=g(function(){let e=g(function(e){return S[e].processed},`compileTask`),t=!0;for(let[n,r]of S.entries())e(n),t&&=r.processed;return t},`compileTasks`),j={getConfig:g(()=>u().journey,`getConfig`),clear:C,setDiagramTitle:f,getDiagramTitle:a,setAccTitle:s,getAccTitle:p,setAccDescription:o,getAccDescription:c,addSection:w,getSections:T,getTasks:E,addTask:O,addTaskOrg:k,getActors:g(function(){return D()},`getActors`)},M=g(t=>`.label {
    font-family: ${t.fontFamily};
    color: ${t.textColor};
  }
  .mouth {
    stroke: #666;
  }

  line {
    stroke: ${t.textColor}
  }

  .legend {
    fill: ${t.textColor};
    font-family: ${t.fontFamily};
  }

  .label text {
    fill: #333;
  }
  .label {
    color: ${t.textColor}
  }

  .face {
    ${t.faceColor?`fill: ${t.faceColor}`:`fill: #FFF8DC`};
    stroke: #999;
  }

  .node rect,
  .node circle,
  .node ellipse,
  .node polygon,
  .node path {
    fill: ${t.mainBkg};
    stroke: ${t.nodeBorder};
    stroke-width: 1px;
  }

  .node .label {
    text-align: center;
  }
  .node.clickable {
    cursor: pointer;
  }

  .arrowheadPath {
    fill: ${t.arrowheadColor};
  }

  .edgePaths .path {
    stroke: ${t.lineColor};
    stroke-width: 1.5px;
  }

  .flowchart-link {
    stroke: ${t.lineColor};
    fill: none;
  }

  .edgeLabel {
    background-color: ${t.edgeLabelBackground};
    rect {
      opacity: 0.5;
    }
    text-align: center;
  }

  .cluster rect {
  }

  .cluster text {
    fill: ${t.titleColor};
  }

  div.mermaidTooltip {
    position: absolute;
    text-align: center;
    max-width: 200px;
    padding: 2px;
    font-family: ${t.fontFamily};
    font-size: 12px;
    background: ${t.tertiaryColor};
    border: 1px solid ${t.border2};
    border-radius: 2px;
    pointer-events: none;
    z-index: 100;
  }

  .task-type-0, .section-type-0  {
    ${t.fillType0?`fill: ${t.fillType0}`:``};
  }
  .task-type-1, .section-type-1  {
    ${t.fillType0?`fill: ${t.fillType1}`:``};
  }
  .task-type-2, .section-type-2  {
    ${t.fillType0?`fill: ${t.fillType2}`:``};
  }
  .task-type-3, .section-type-3  {
    ${t.fillType0?`fill: ${t.fillType3}`:``};
  }
  .task-type-4, .section-type-4  {
    ${t.fillType0?`fill: ${t.fillType4}`:``};
  }
  .task-type-5, .section-type-5  {
    ${t.fillType0?`fill: ${t.fillType5}`:``};
  }
  .task-type-6, .section-type-6  {
    ${t.fillType0?`fill: ${t.fillType6}`:``};
  }
  .task-type-7, .section-type-7  {
    ${t.fillType0?`fill: ${t.fillType7}`:``};
  }

  .actor-0 {
    ${t.actor0?`fill: ${t.actor0}`:``};
  }
  .actor-1 {
    ${t.actor1?`fill: ${t.actor1}`:``};
  }
  .actor-2 {
    ${t.actor2?`fill: ${t.actor2}`:``};
  }
  .actor-3 {
    ${t.actor3?`fill: ${t.actor3}`:``};
  }
  .actor-4 {
    ${t.actor4?`fill: ${t.actor4}`:``};
  }
  .actor-5 {
    ${t.actor5?`fill: ${t.actor5}`:``};
  }
  ${e()}
`,`getStyles`),N=g(function(e,n){return t(e,n)},`drawRect`),P=g(function(e,t){let n=e.append(`circle`).attr(`cx`,t.cx).attr(`cy`,t.cy).attr(`class`,`face`).attr(`r`,15).attr(`stroke-width`,2).attr(`overflow`,`visible`),r=e.append(`g`);r.append(`circle`).attr(`cx`,t.cx-5).attr(`cy`,t.cy-5).attr(`r`,1.5).attr(`stroke-width`,2).attr(`fill`,`#666`).attr(`stroke`,`#666`),r.append(`circle`).attr(`cx`,t.cx+5).attr(`cy`,t.cy-5).attr(`r`,1.5).attr(`stroke-width`,2).attr(`fill`,`#666`).attr(`stroke`,`#666`);function i(e){let n=m().startAngle(Math.PI/2).endAngle(Math.PI/2*3).innerRadius(15/2).outerRadius(15/2.2);e.append(`path`).attr(`class`,`mouth`).attr(`d`,n).attr(`transform`,`translate(`+t.cx+`,`+(t.cy+2)+`)`)}g(i,`smile`);function a(e){let n=m().startAngle(3*Math.PI/2).endAngle(Math.PI/2*5).innerRadius(15/2).outerRadius(15/2.2);e.append(`path`).attr(`class`,`mouth`).attr(`d`,n).attr(`transform`,`translate(`+t.cx+`,`+(t.cy+7)+`)`)}g(a,`sad`);function o(e){e.append(`line`).attr(`class`,`mouth`).attr(`stroke`,2).attr(`x1`,t.cx-5).attr(`y1`,t.cy+7).attr(`x2`,t.cx+5).attr(`y2`,t.cy+7).attr(`class`,`mouth`).attr(`stroke-width`,`1px`).attr(`stroke`,`#666`)}return g(o,`ambivalent`),t.score>3?i(r):t.score<3?a(r):o(r),n},`drawFace`),F=g(function(e,t){let n=e.append(`circle`);return n.attr(`cx`,t.cx),n.attr(`cy`,t.cy),n.attr(`class`,`actor-`+t.pos),n.attr(`fill`,t.fill),n.attr(`stroke`,t.stroke),n.attr(`r`,t.r),n.class!==void 0&&n.attr(`class`,n.class),t.title!==void 0&&n.append(`title`).text(t.title),n},`drawCircle`),I=g(function(e,t){return r(e,t)},`drawText`),L=g(function(e,t){function n(e,t,n,r,i){return e+`,`+t+` `+(e+n)+`,`+t+` `+(e+n)+`,`+(t+r-i)+` `+(e+n-i*1.2)+`,`+(t+r)+` `+e+`,`+(t+r)}g(n,`genPoints`);let r=e.append(`polygon`);r.attr(`points`,n(t.x,t.y,50,20,7)),r.attr(`class`,`labelBox`),t.y+=t.labelMargin,t.x+=.5*t.labelMargin,I(e,t)},`drawLabel`),R=g(function(e,t,n){let r=e.append(`g`),a=i();a.x=t.x,a.y=t.y,a.fill=t.fill,a.width=n.width*t.taskCount+n.diagramMarginX*(t.taskCount-1),a.height=n.height,a.class=`journey-section section-type-`+t.num,a.rx=3,a.ry=3,N(r,a),H(n)(t.text,r,a.x,a.y,a.width,a.height,{class:`journey-section section-type-`+t.num},n,t.colour)},`drawSection`),z=-1,B=g(function(e,t,n,r){let a=t.x+n.width/2,o=e.append(`g`);z++,o.append(`line`).attr(`id`,r+`-task`+z).attr(`x1`,a).attr(`y1`,t.y).attr(`x2`,a).attr(`y2`,450).attr(`class`,`task-line`).attr(`stroke-width`,`1px`).attr(`stroke-dasharray`,`4 2`).attr(`stroke`,`#666`),P(o,{cx:a,cy:300+(5-t.score)*30,score:t.score});let s=i();s.x=t.x,s.y=t.y,s.fill=t.fill,s.width=n.width,s.height=n.height,s.class=`task task-type-`+t.num,s.rx=3,s.ry=3,N(o,s);let c=t.x+14;t.people.forEach(e=>{let n=t.actors[e].color,r={cx:c,cy:t.y,r:7,fill:n,stroke:`#000`,title:e,pos:t.actors[e].position};F(o,r),c+=10}),H(n)(t.task,o,s.x,s.y,s.width,s.height,{class:`task`},n,t.colour)},`drawTask`),V=g(function(e,t){n(e,t)},`drawBackgroundRect`),H=(function(){function e(e,t,n,i,a,o,s,c){r(t.append(`text`).attr(`x`,n+a/2).attr(`y`,i+o/2+5).style(`font-color`,c).style(`text-anchor`,`middle`).text(e),s)}g(e,`byText`);function t(e,t,n,i,a,o,s,c,l){let{taskFontSize:u,taskFontFamily:d}=c,f=e.split(/<br\s*\/?>/gi);for(let e=0;e<f.length;e++){let c=e*u-u*(f.length-1)/2,p=t.append(`text`).attr(`x`,n+a/2).attr(`y`,i).attr(`fill`,l).style(`text-anchor`,`middle`).style(`font-size`,u).style(`font-family`,d);p.append(`tspan`).attr(`x`,n+a/2).attr(`dy`,c).text(f[e]),p.attr(`y`,i+o/2).attr(`dominant-baseline`,`central`).attr(`alignment-baseline`,`central`),r(p,s)}}g(t,`byTspan`);function n(e,n,i,a,o,s,c,l){let u=n.append(`switch`),d=u.append(`foreignObject`).attr(`x`,i).attr(`y`,a).attr(`width`,o).attr(`height`,s).attr(`position`,`fixed`).append(`xhtml:div`).style(`display`,`table`).style(`height`,`100%`).style(`width`,`100%`);d.append(`div`).attr(`class`,`label`).style(`display`,`table-cell`).style(`text-align`,`center`).style(`vertical-align`,`middle`).text(e),t(e,u,i,a,o,s,c,l),r(d,c)}g(n,`byFo`);function r(e,t){for(let n in t)n in t&&e.attr(n,t[n])}return g(r,`_setTextAttrs`),function(r){return r.textPlacement===`fo`?n:r.textPlacement===`old`?e:t}})(),U={drawRect:N,drawCircle:F,drawSection:R,drawText:I,drawLabel:L,drawTask:B,drawBackgroundRect:V,initGraphics:g(function(e,t){z=-1,e.append(`defs`).append(`marker`).attr(`id`,t+`-arrowhead`).attr(`refX`,5).attr(`refY`,2).attr(`markerWidth`,6).attr(`markerHeight`,4).attr(`orient`,`auto`).append(`path`).attr(`d`,`M 0,0 V 4 L6,2 Z`)},`initGraphics`)},W=g(function(e){Object.keys(e).forEach(function(t){J[t]=e[t]})},`setConf`),G={},K=0;function q(e){let t=u().journey,n=t.maxLabelWidth;K=0;let r=60;Object.keys(G).forEach(i=>{let a=G[i].color,o={cx:20,cy:r,r:7,fill:a,stroke:`#000`,pos:G[i].position};U.drawCircle(e,o);let s=e.append(`text`).attr(`visibility`,`hidden`).text(i),c=s.node().getBoundingClientRect().width;s.remove();let l=[];if(c<=n)l=[i];else{let t=i.split(` `),r=``;s=e.append(`text`).attr(`visibility`,`hidden`),t.forEach(e=>{let t=r?`${r} ${e}`:e;if(s.text(t),s.node().getBoundingClientRect().width>n){if(r&&l.push(r),r=e,s.text(e),s.node().getBoundingClientRect().width>n){let t=``;for(let r of e)t+=r,s.text(t+`-`),s.node().getBoundingClientRect().width>n&&(l.push(t.slice(0,-1)+`-`),t=r);r=t}}else r=t}),r&&l.push(r),s.remove()}l.forEach((n,i)=>{let a={x:40,y:r+7+i*20,fill:`#666`,text:n,textMargin:t.boxTextMargin??5},o=U.drawText(e,a).node().getBoundingClientRect().width;o>K&&o>t.leftMargin-o&&(K=o)}),r+=Math.max(20,l.length*20)})}g(q,`drawActorLegend`);var J=u().journey,Y=0,ee=g(function(e,t,n,r){let i=u(),a=i.journey.titleColor,o=i.journey.titleFontSize,s=i.journey.titleFontFamily,c=i.securityLevel,l;c===`sandbox`&&(l=h(`#i`+t));let f=h(c===`sandbox`?l.nodes()[0].contentDocument.body:`body`);X.init();let p=f.select(`#`+t);U.initGraphics(p,t);let m=r.db.getTasks(),g=r.db.getDiagramTitle(),_=r.db.getActors();for(let e in G)delete G[e];let v=0;_.forEach(e=>{G[e]={color:J.actorColours[v%J.actorColours.length],position:v},v++}),q(p),Y=J.leftMargin+K,X.insert(0,0,Y,Object.keys(G).length*50),te(p,m,0,t);let y=X.getBounds();g&&p.append(`text`).text(g).attr(`x`,Y).attr(`font-size`,o).attr(`font-weight`,`bold`).attr(`y`,25).attr(`fill`,a).attr(`font-family`,s);let b=y.stopy-y.starty+2*J.diagramMarginY,x=Y+y.stopx+2*J.diagramMarginX;d(p,b,x,J.useMaxWidth),p.append(`line`).attr(`x1`,Y).attr(`y1`,J.height*4).attr(`x2`,x-Y-4).attr(`y2`,J.height*4).attr(`stroke-width`,4).attr(`stroke`,`black`).attr(`marker-end`,`url(#`+t+`-arrowhead)`);let S=g?70:0;p.attr(`viewBox`,`${y.startx} -25 ${x} ${b+S}`),p.attr(`preserveAspectRatio`,`xMinYMin meet`),p.attr(`height`,b+S+25)},`draw`),X={data:{startx:void 0,stopx:void 0,starty:void 0,stopy:void 0},verticalPos:0,sequenceItems:[],init:g(function(){this.sequenceItems=[],this.data={startx:void 0,stopx:void 0,starty:void 0,stopy:void 0},this.verticalPos=0},`init`),updateVal:g(function(e,t,n,r){e[t]=e[t]===void 0?n:r(n,e[t])},`updateVal`),updateBounds:g(function(e,t,n,r){let i=u().journey,a=this,o=0;function s(s){return g(function(c){o++;let l=a.sequenceItems.length-o+1;a.updateVal(c,`starty`,t-l*i.boxMargin,Math.min),a.updateVal(c,`stopy`,r+l*i.boxMargin,Math.max),a.updateVal(X.data,`startx`,e-l*i.boxMargin,Math.min),a.updateVal(X.data,`stopx`,n+l*i.boxMargin,Math.max),s!==`activation`&&(a.updateVal(c,`startx`,e-l*i.boxMargin,Math.min),a.updateVal(c,`stopx`,n+l*i.boxMargin,Math.max),a.updateVal(X.data,`starty`,t-l*i.boxMargin,Math.min),a.updateVal(X.data,`stopy`,r+l*i.boxMargin,Math.max))},`updateItemBounds`)}g(s,`updateFn`),this.sequenceItems.forEach(s())},`updateBounds`),insert:g(function(e,t,n,r){let i=Math.min(e,n),a=Math.max(e,n),o=Math.min(t,r),s=Math.max(t,r);this.updateVal(X.data,`startx`,i,Math.min),this.updateVal(X.data,`starty`,o,Math.min),this.updateVal(X.data,`stopx`,a,Math.max),this.updateVal(X.data,`stopy`,s,Math.max),this.updateBounds(i,o,a,s)},`insert`),bumpVerticalPos:g(function(e){this.verticalPos+=e,this.data.stopy=this.verticalPos},`bumpVerticalPos`),getVerticalPos:g(function(){return this.verticalPos},`getVerticalPos`),getBounds:g(function(){return this.data},`getBounds`)},Z=J.sectionFills,Q=J.sectionColours,te=g(function(e,t,n,r){let i=u().journey,a=``,o=n+(i.height*2+i.diagramMarginY),s=0,c=`#CCC`,l=`black`,d=0;for(let[n,u]of t.entries()){if(a!==u.section){c=Z[s%Z.length],d=s%Z.length,l=Q[s%Q.length];let r=0,o=u.section;for(let e=n;e<t.length&&t[e].section==o;e++)r+=1;let f={x:n*i.taskMargin+n*i.width+Y,y:50,text:u.section,fill:c,num:d,colour:l,taskCount:r};U.drawSection(e,f,i),a=u.section,s++}let f=u.people.reduce((e,t)=>(G[t]&&(e[t]=G[t]),e),{});u.x=n*i.taskMargin+n*i.width+Y,u.y=o,u.width=i.diagramMarginX,u.height=i.diagramMarginY,u.colour=l,u.fill=c,u.num=d,u.actors=f,U.drawTask(e,u,i,r),X.insert(u.x,u.y,u.x+u.width+i.taskMargin,450)}},`drawTasks`),$={setConf:W,draw:ee},ne={parser:v,db:j,renderer:$,styles:M,init:g(e=>{$.setConf(e.journey),j.clear()},`init`)};export{ne as diagram};