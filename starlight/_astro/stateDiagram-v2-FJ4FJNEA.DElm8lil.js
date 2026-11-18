import{t as e}from"./chunk-XXDRQBXY.HmlYESbh.js";import{t}from"./chunk-TU62E6Q5.361HbaxT.js";import{i as n,r}from"./mermaid.core.Bf0pvaT3.js";import{t as i}from"./chunk-HFQBQIA3.7oKsMhA6.js";import"./chunk-M57EVK23.NTGFgixz.js";import"./chunk-DQASFODM.51Me_6no.js";import"./chunk-LEVMRMFU.DNsRHphg.js";import"./chunk-AXFCVDMN.8rhIwtQV.js";import"./chunk-X4PVCEUW.D8YY5SaX.js";import"./chunk-STOXU5M4.UOUNWiwh.js";import"./chunk-R3KSYZAL.BBY1WNjz.js";import{a,i as o,o as s,r as c}from"./chunk-J5ZVWO5B.Ddht6hlJ.js";import{_ as l,s as u}from"./chunk-6MUBBAK2.CfXUfaG5.js";import{C as d,H as f,U as p,_ as m,a as h,b as g,et as _,q as v,s as y,v as b}from"./chunk-BG4553SI.DzMjcvML.js";import{m as x,p as S}from"./src.Duoub3vK.js";import{n as C}from"./chunk-Y2CYZVJY.DsF7k-Jl.js";var w=(function(){var e=C(function(e,t,n,r){for(n||={},r=e.length;r--;n[e[r]]=t);return n},`o`),t=[1,2],n=[1,3],r=[1,4],i=[2,4],a=[1,9],o=[1,11],s=[1,16],c=[1,17],l=[1,18],u=[1,19],d=[1,33],f=[1,20],p=[1,21],m=[1,22],h=[1,23],g=[1,24],_=[1,26],v=[1,27],y=[1,28],b=[1,29],x=[1,30],S=[1,31],w=[1,32],T=[1,35],E=[1,36],D=[1,37],O=[1,38],k=[1,34],A=[1,4,5,16,17,19,21,22,24,25,26,27,28,29,33,35,37,38,41,45,48,51,52,53,54,57],j=[1,4,5,14,15,16,17,19,21,22,24,25,26,27,28,29,33,35,37,38,39,40,41,45,48,51,52,53,54,57],M=[4,5,16,17,19,21,22,24,25,26,27,28,29,33,35,37,38,41,45,48,51,52,53,54,57],N={trace:C(function(){},`trace`),yy:{},symbols_:{error:2,start:3,SPACE:4,NL:5,SD:6,document:7,line:8,statement:9,classDefStatement:10,styleStatement:11,cssClassStatement:12,idStatement:13,DESCR:14,"-->":15,HIDE_EMPTY:16,scale:17,WIDTH:18,COMPOSIT_STATE:19,STRUCT_START:20,STRUCT_STOP:21,STATE_DESCR:22,AS:23,ID:24,FORK:25,JOIN:26,CHOICE:27,CONCURRENT:28,note:29,notePosition:30,NOTE_TEXT:31,direction:32,acc_title:33,acc_title_value:34,acc_descr:35,acc_descr_value:36,acc_descr_multiline_value:37,CLICK:38,STRING:39,HREF:40,classDef:41,CLASSDEF_ID:42,CLASSDEF_STYLEOPTS:43,DEFAULT:44,style:45,STYLE_IDS:46,STYLEDEF_STYLEOPTS:47,class:48,CLASSENTITY_IDS:49,STYLECLASS:50,direction_tb:51,direction_bt:52,direction_rl:53,direction_lr:54,eol:55,";":56,EDGE_STATE:57,STYLE_SEPARATOR:58,left_of:59,right_of:60,$accept:0,$end:1},terminals_:{2:`error`,4:`SPACE`,5:`NL`,6:`SD`,14:`DESCR`,15:`-->`,16:`HIDE_EMPTY`,17:`scale`,18:`WIDTH`,19:`COMPOSIT_STATE`,20:`STRUCT_START`,21:`STRUCT_STOP`,22:`STATE_DESCR`,23:`AS`,24:`ID`,25:`FORK`,26:`JOIN`,27:`CHOICE`,28:`CONCURRENT`,29:`note`,31:`NOTE_TEXT`,33:`acc_title`,34:`acc_title_value`,35:`acc_descr`,36:`acc_descr_value`,37:`acc_descr_multiline_value`,38:`CLICK`,39:`STRING`,40:`HREF`,41:`classDef`,42:`CLASSDEF_ID`,43:`CLASSDEF_STYLEOPTS`,44:`DEFAULT`,45:`style`,46:`STYLE_IDS`,47:`STYLEDEF_STYLEOPTS`,48:`class`,49:`CLASSENTITY_IDS`,50:`STYLECLASS`,51:`direction_tb`,52:`direction_bt`,53:`direction_rl`,54:`direction_lr`,56:`;`,57:`EDGE_STATE`,58:`STYLE_SEPARATOR`,59:`left_of`,60:`right_of`},productions_:[0,[3,2],[3,2],[3,2],[7,0],[7,2],[8,2],[8,1],[8,1],[9,1],[9,1],[9,1],[9,1],[9,2],[9,3],[9,4],[9,1],[9,2],[9,1],[9,4],[9,3],[9,6],[9,1],[9,1],[9,1],[9,1],[9,4],[9,4],[9,1],[9,2],[9,2],[9,1],[9,5],[9,5],[10,3],[10,3],[11,3],[12,3],[32,1],[32,1],[32,1],[32,1],[55,1],[55,1],[13,1],[13,1],[13,3],[13,3],[30,1],[30,1]],performAction:C(function(e,t,n,r,i,a,o){var s=a.length-1;switch(i){case 3:return r.setRootDoc(a[s]),a[s];case 4:this.$=[];break;case 5:a[s]!=`nl`&&(a[s-1].push(a[s]),this.$=a[s-1]);break;case 6:case 7:this.$=a[s];break;case 8:this.$=`nl`;break;case 12:this.$=a[s];break;case 13:let e=a[s-1];e.description=r.trimColon(a[s]),this.$=e;break;case 14:this.$={stmt:`relation`,state1:a[s-2],state2:a[s]};break;case 15:let t=r.trimColon(a[s]);this.$={stmt:`relation`,state1:a[s-3],state2:a[s-1],description:t};break;case 19:this.$={stmt:`state`,id:a[s-3],type:`default`,description:``,doc:a[s-1]};break;case 20:var c=a[s],l=a[s-2].trim();if(a[s].match(`:`)){var u=a[s].split(`:`);c=u[0],l=[l,u[1]]}this.$={stmt:`state`,id:c,type:`default`,description:l};break;case 21:this.$={stmt:`state`,id:a[s-3],type:`default`,description:a[s-5],doc:a[s-1]};break;case 22:this.$={stmt:`state`,id:a[s],type:`fork`};break;case 23:this.$={stmt:`state`,id:a[s],type:`join`};break;case 24:this.$={stmt:`state`,id:a[s],type:`choice`};break;case 25:this.$={stmt:`state`,id:r.getDividerId(),type:`divider`};break;case 26:this.$={stmt:`state`,id:a[s-1].trim(),note:{position:a[s-2].trim(),text:a[s].trim()}};break;case 29:this.$=a[s].trim(),r.setAccTitle(this.$);break;case 30:case 31:this.$=a[s].trim(),r.setAccDescription(this.$);break;case 32:this.$={stmt:`click`,id:a[s-3],url:a[s-2],tooltip:a[s-1]};break;case 33:this.$={stmt:`click`,id:a[s-3],url:a[s-1],tooltip:``};break;case 34:case 35:this.$={stmt:`classDef`,id:a[s-1].trim(),classes:a[s].trim()};break;case 36:this.$={stmt:`style`,id:a[s-1].trim(),styleClass:a[s].trim()};break;case 37:this.$={stmt:`applyClass`,id:a[s-1].trim(),styleClass:a[s].trim()};break;case 38:r.setDirection(`TB`),this.$={stmt:`dir`,value:`TB`};break;case 39:r.setDirection(`BT`),this.$={stmt:`dir`,value:`BT`};break;case 40:r.setDirection(`RL`),this.$={stmt:`dir`,value:`RL`};break;case 41:r.setDirection(`LR`),this.$={stmt:`dir`,value:`LR`};break;case 44:case 45:this.$={stmt:`state`,id:a[s].trim(),type:`default`,description:``};break;case 46:this.$={stmt:`state`,id:a[s-2].trim(),classes:[a[s].trim()],type:`default`,description:``};break;case 47:this.$={stmt:`state`,id:a[s-2].trim(),classes:[a[s].trim()],type:`default`,description:``}}},`anonymous`),table:[{3:1,4:t,5:n,6:r},{1:[3]},{3:5,4:t,5:n,6:r},{3:6,4:t,5:n,6:r},e([1,4,5,16,17,19,22,24,25,26,27,28,29,33,35,37,38,41,45,48,51,52,53,54,57],i,{7:7}),{1:[2,1]},{1:[2,2]},{1:[2,3],4:a,5:o,8:8,9:10,10:12,11:13,12:14,13:15,16:s,17:c,19:l,22:u,24:d,25:f,26:p,27:m,28:h,29:g,32:25,33:_,35:v,37:y,38:b,41:x,45:S,48:w,51:T,52:E,53:D,54:O,57:k},e(A,[2,5]),{9:39,10:12,11:13,12:14,13:15,16:s,17:c,19:l,22:u,24:d,25:f,26:p,27:m,28:h,29:g,32:25,33:_,35:v,37:y,38:b,41:x,45:S,48:w,51:T,52:E,53:D,54:O,57:k},e(A,[2,7]),e(A,[2,8]),e(A,[2,9]),e(A,[2,10]),e(A,[2,11]),e(A,[2,12],{14:[1,40],15:[1,41]}),e(A,[2,16]),{18:[1,42]},e(A,[2,18],{20:[1,43]}),{23:[1,44]},e(A,[2,22]),e(A,[2,23]),e(A,[2,24]),e(A,[2,25]),{30:45,31:[1,46],59:[1,47],60:[1,48]},e(A,[2,28]),{34:[1,49]},{36:[1,50]},e(A,[2,31]),{13:51,24:d,57:k},{42:[1,52],44:[1,53]},{46:[1,54]},{49:[1,55]},e(j,[2,44],{58:[1,56]}),e(j,[2,45],{58:[1,57]}),e(A,[2,38]),e(A,[2,39]),e(A,[2,40]),e(A,[2,41]),e(A,[2,6]),e(A,[2,13]),{13:58,24:d,57:k},e(A,[2,17]),e(M,i,{7:59}),{24:[1,60]},{24:[1,61]},{23:[1,62]},{24:[2,48]},{24:[2,49]},e(A,[2,29]),e(A,[2,30]),{39:[1,63],40:[1,64]},{43:[1,65]},{43:[1,66]},{47:[1,67]},{50:[1,68]},{24:[1,69]},{24:[1,70]},e(A,[2,14],{14:[1,71]}),{4:a,5:o,8:8,9:10,10:12,11:13,12:14,13:15,16:s,17:c,19:l,21:[1,72],22:u,24:d,25:f,26:p,27:m,28:h,29:g,32:25,33:_,35:v,37:y,38:b,41:x,45:S,48:w,51:T,52:E,53:D,54:O,57:k},e(A,[2,20],{20:[1,73]}),{31:[1,74]},{24:[1,75]},{39:[1,76]},{39:[1,77]},e(A,[2,34]),e(A,[2,35]),e(A,[2,36]),e(A,[2,37]),e(j,[2,46]),e(j,[2,47]),e(A,[2,15]),e(A,[2,19]),e(M,i,{7:78}),e(A,[2,26]),e(A,[2,27]),{5:[1,79]},{5:[1,80]},{4:a,5:o,8:8,9:10,10:12,11:13,12:14,13:15,16:s,17:c,19:l,21:[1,81],22:u,24:d,25:f,26:p,27:m,28:h,29:g,32:25,33:_,35:v,37:y,38:b,41:x,45:S,48:w,51:T,52:E,53:D,54:O,57:k},e(A,[2,32]),e(A,[2,33]),e(A,[2,21])],defaultActions:{5:[2,1],6:[2,2],47:[2,48],48:[2,49]},parseError:C(function(e,t){if(t.recoverable)this.trace(e);else{var n=Error(e);throw n.hash=t,n}},`parseError`),parse:C(function(e){var t=this,n=[0],r=[],i=[null],a=[],o=this.table,s=``,c=0,l=0,u=0,d=2,f=1,p=a.slice.call(arguments,1),m=Object.create(this.lexer),h={yy:{}};for(var g in this.yy)Object.prototype.hasOwnProperty.call(this.yy,g)&&(h.yy[g]=this.yy[g]);m.setInput(e,h.yy),h.yy.lexer=m,h.yy.parser=this,m.yylloc===void 0&&(m.yylloc={});var _=m.yylloc;a.push(_);var v=m.options&&m.options.ranges;this.parseError=typeof h.yy.parseError==`function`?h.yy.parseError:Object.getPrototypeOf(this).parseError;function y(e){n.length-=2*e,i.length-=e,a.length-=e}C(y,`popStack`);function b(){var e=r.pop()||m.lex()||f;return typeof e!=`number`&&(e instanceof Array&&(r=e,e=r.pop()),e=t.symbols_[e]||e),e}C(b,`lex`);for(var x,S,w,T,E,D={},O,k,A,j;;){if(w=n[n.length-1],this.defaultActions[w]?T=this.defaultActions[w]:(x??=b(),T=o[w]&&o[w][x]),T===void 0||!T.length||!T[0]){var M=``;for(O in j=[],o[w])this.terminals_[O]&&O>d&&j.push(`'`+this.terminals_[O]+`'`);M=m.showPosition?`Parse error on line `+(c+1)+`:
`+m.showPosition()+`
Expecting `+j.join(`, `)+`, got '`+(this.terminals_[x]||x)+`'`:`Parse error on line `+(c+1)+`: Unexpected `+(x==f?`end of input`:`'`+(this.terminals_[x]||x)+`'`),this.parseError(M,{text:m.match,token:this.terminals_[x]||x,line:m.yylineno,loc:_,expected:j})}if(T[0]instanceof Array&&T.length>1)throw Error(`Parse Error: multiple actions possible at state: `+w+`, token: `+x);switch(T[0]){case 1:n.push(x),i.push(m.yytext),a.push(m.yylloc),n.push(T[1]),x=null,S?(x=S,S=null):(l=m.yyleng,s=m.yytext,c=m.yylineno,_=m.yylloc,u>0&&u--);break;case 2:if(k=this.productions_[T[1]][1],D.$=i[i.length-k],D._$={first_line:a[a.length-(k||1)].first_line,last_line:a[a.length-1].last_line,first_column:a[a.length-(k||1)].first_column,last_column:a[a.length-1].last_column},v&&(D._$.range=[a[a.length-(k||1)].range[0],a[a.length-1].range[1]]),E=this.performAction.apply(D,[s,l,c,h.yy,T[1],i,a].concat(p)),E!==void 0)return E;k&&(n=n.slice(0,-1*k*2),i=i.slice(0,-1*k),a=a.slice(0,-1*k)),n.push(this.productions_[T[1]][0]),i.push(D.$),a.push(D._$),A=o[n[n.length-2]][n[n.length-1]],n.push(A);break;case 3:return!0}}return!0},`parse`)};N.lexer=(function(){return{EOF:1,parseError:C(function(e,t){if(this.yy.parser)this.yy.parser.parseError(e,t);else throw Error(e)},`parseError`),setInput:C(function(e,t){return this.yy=t||this.yy||{},this._input=e,this._more=this._backtrack=this.done=!1,this.yylineno=this.yyleng=0,this.yytext=this.matched=this.match=``,this.conditionStack=[`INITIAL`],this.yylloc={first_line:1,first_column:0,last_line:1,last_column:0},this.options.ranges&&(this.yylloc.range=[0,0]),this.offset=0,this},`setInput`),input:C(function(){var e=this._input[0];return this.yytext+=e,this.yyleng++,this.offset++,this.match+=e,this.matched+=e,e.match(/(?:\r\n?|\n).*/g)?(this.yylineno++,this.yylloc.last_line++):this.yylloc.last_column++,this.options.ranges&&this.yylloc.range[1]++,this._input=this._input.slice(1),e},`input`),unput:C(function(e){var t=e.length,n=e.split(/(?:\r\n?|\n)/g);this._input=e+this._input,this.yytext=this.yytext.substr(0,this.yytext.length-t),this.offset-=t;var r=this.match.split(/(?:\r\n?|\n)/g);this.match=this.match.substr(0,this.match.length-1),this.matched=this.matched.substr(0,this.matched.length-1),n.length-1&&(this.yylineno-=n.length-1);var i=this.yylloc.range;return this.yylloc={first_line:this.yylloc.first_line,last_line:this.yylineno+1,first_column:this.yylloc.first_column,last_column:n?(n.length===r.length?this.yylloc.first_column:0)+r[r.length-n.length].length-n[0].length:this.yylloc.first_column-t},this.options.ranges&&(this.yylloc.range=[i[0],i[0]+this.yyleng-t]),this.yyleng=this.yytext.length,this},`unput`),more:C(function(){return this._more=!0,this},`more`),reject:C(function(){if(this.options.backtrack_lexer)this._backtrack=!0;else return this.parseError(`Lexical error on line `+(this.yylineno+1)+`. You can only invoke reject() in the lexer when the lexer is of the backtracking persuasion (options.backtrack_lexer = true).
`+this.showPosition(),{text:``,token:null,line:this.yylineno});return this},`reject`),less:C(function(e){this.unput(this.match.slice(e))},`less`),pastInput:C(function(){var e=this.matched.substr(0,this.matched.length-this.match.length);return(e.length>20?`...`:``)+e.substr(-20).replace(/\n/g,``)},`pastInput`),upcomingInput:C(function(){var e=this.match;return e.length<20&&(e+=this._input.substr(0,20-e.length)),(e.substr(0,20)+(e.length>20?`...`:``)).replace(/\n/g,``)},`upcomingInput`),showPosition:C(function(){var e=this.pastInput(),t=Array(e.length+1).join(`-`);return e+this.upcomingInput()+`
`+t+`^`},`showPosition`),test_match:C(function(e,t){var n,r,i;if(this.options.backtrack_lexer&&(i={yylineno:this.yylineno,yylloc:{first_line:this.yylloc.first_line,last_line:this.last_line,first_column:this.yylloc.first_column,last_column:this.yylloc.last_column},yytext:this.yytext,match:this.match,matches:this.matches,matched:this.matched,yyleng:this.yyleng,offset:this.offset,_more:this._more,_input:this._input,yy:this.yy,conditionStack:this.conditionStack.slice(0),done:this.done},this.options.ranges&&(i.yylloc.range=this.yylloc.range.slice(0))),r=e[0].match(/(?:\r\n?|\n).*/g),r&&(this.yylineno+=r.length),this.yylloc={first_line:this.yylloc.last_line,last_line:this.yylineno+1,first_column:this.yylloc.last_column,last_column:r?r[r.length-1].length-r[r.length-1].match(/\r?\n?/)[0].length:this.yylloc.last_column+e[0].length},this.yytext+=e[0],this.match+=e[0],this.matches=e,this.yyleng=this.yytext.length,this.options.ranges&&(this.yylloc.range=[this.offset,this.offset+=this.yyleng]),this._more=!1,this._backtrack=!1,this._input=this._input.slice(e[0].length),this.matched+=e[0],n=this.performAction.call(this,this.yy,this,t,this.conditionStack[this.conditionStack.length-1]),this.done&&this._input&&(this.done=!1),n)return n;if(this._backtrack){for(var a in i)this[a]=i[a];return!1}return!1},`test_match`),next:C(function(){if(this.done)return this.EOF;this._input||(this.done=!0);var e,t,n,r;this._more||(this.yytext=``,this.match=``);for(var i=this._currentRules(),a=0;a<i.length;a++)if(n=this._input.match(this.rules[i[a]]),n&&(!t||n[0].length>t[0].length)){if(t=n,r=a,this.options.backtrack_lexer){if(e=this.test_match(n,i[a]),e!==!1)return e;if(this._backtrack){t=!1;continue}return!1}if(!this.options.flex)break}return t?(e=this.test_match(t,i[r]),e!==!1&&e):this._input===``?this.EOF:this.parseError(`Lexical error on line `+(this.yylineno+1)+`. Unrecognized text.
`+this.showPosition(),{text:``,token:null,line:this.yylineno})},`next`),lex:C(function(){return this.next()||this.lex()},`lex`),begin:C(function(e){this.conditionStack.push(e)},`begin`),popState:C(function(){return this.conditionStack.length-1>0?this.conditionStack.pop():this.conditionStack[0]},`popState`),_currentRules:C(function(){return this.conditionStack.length&&this.conditionStack[this.conditionStack.length-1]?this.conditions[this.conditionStack[this.conditionStack.length-1]].rules:this.conditions.INITIAL.rules},`_currentRules`),topState:C(function(e){return e=this.conditionStack.length-1-Math.abs(e||0),e>=0?this.conditionStack[e]:`INITIAL`},`topState`),pushState:C(function(e){this.begin(e)},`pushState`),stateStackSize:C(function(){return this.conditionStack.length},`stateStackSize`),options:{"case-insensitive":!0},performAction:C(function(e,t,n,r){function i(){let n=t.yytext.indexOf(`%%`);if(n===0)return!1;if(n>0){let r=t.yytext.slice(0,n),i=t.yytext.slice(n);i&&e.lexer.unput(i),t.yytext=r}return!0}switch(C(i,`processId`),n){case 0:return 38;case 1:return 40;case 2:return 39;case 3:return 44;case 4:return 51;case 5:return 52;case 6:return 53;case 7:return 54;case 8:return 5;case 9:break;case 10:break;case 11:break;case 12:break;case 13:return this.pushState(`SCALE`),17;case 14:return 18;case 15:this.popState();break;case 16:return this.begin(`acc_title`),33;case 17:return this.popState(),`acc_title_value`;case 18:return this.begin(`acc_descr`),35;case 19:return this.popState(),`acc_descr_value`;case 20:this.begin(`acc_descr_multiline`);break;case 21:this.popState();break;case 22:return`acc_descr_multiline_value`;case 23:return this.pushState(`CLASSDEF`),41;case 24:return this.popState(),this.pushState(`CLASSDEFID`),`DEFAULT_CLASSDEF_ID`;case 25:return this.popState(),this.pushState(`CLASSDEFID`),42;case 26:return this.popState(),43;case 27:return this.pushState(`CLASS`),48;case 28:return this.popState(),this.pushState(`CLASS_STYLE`),49;case 29:return this.popState(),50;case 30:return this.pushState(`STYLE`),45;case 31:return this.popState(),this.pushState(`STYLEDEF_STYLES`),46;case 32:return this.popState(),47;case 33:return this.pushState(`SCALE`),17;case 34:return 18;case 35:this.popState();break;case 36:this.pushState(`STATE`);break;case 37:return this.popState(),t.yytext=t.yytext.slice(0,-8).trim(),25;case 38:return this.popState(),t.yytext=t.yytext.slice(0,-8).trim(),26;case 39:return this.popState(),t.yytext=t.yytext.slice(0,-10).trim(),27;case 40:return this.popState(),t.yytext=t.yytext.slice(0,-8).trim(),25;case 41:return this.popState(),t.yytext=t.yytext.slice(0,-8).trim(),26;case 42:return this.popState(),t.yytext=t.yytext.slice(0,-10).trim(),27;case 43:return 51;case 44:return 52;case 45:return 53;case 46:return 54;case 47:this.pushState(`STATE_STRING`);break;case 48:return this.pushState(`STATE_ID`),`AS`;case 49:return i()?(this.popState(),`ID`):void 0;case 50:this.popState();break;case 51:return`STATE_DESCR`;case 52:throw Error(`Error: State name must be a single word. Found: "`+t.yytext.trim()+`"`);case 53:return 19;case 54:this.popState();break;case 55:return this.popState(),this.pushState(`struct`),20;case 56:return this.popState(),21;case 57:break;case 58:return this.begin(`NOTE`),29;case 59:return this.popState(),this.pushState(`NOTE_ID`),59;case 60:return this.popState(),this.pushState(`NOTE_ID`),60;case 61:this.popState(),this.pushState(`FLOATING_NOTE`);break;case 62:return this.popState(),this.pushState(`FLOATING_NOTE_ID`),`AS`;case 63:break;case 64:return`NOTE_TEXT`;case 65:return i()?(this.popState(),`ID`):void 0;case 66:return i()?(this.popState(),this.pushState(`NOTE_TEXT`),24):void 0;case 67:return this.popState(),t.yytext=t.yytext.substr(2).trim(),31;case 68:return this.popState(),t.yytext=t.yytext.slice(0,-8).trim(),31;case 69:return 6;case 70:return 6;case 71:return 16;case 72:return 57;case 73:return i()?24:void 0;case 74:return t.yytext=t.yytext.trim(),14;case 75:return 15;case 76:return 28;case 77:return 58;case 78:return 5;case 79:return`INVALID`}},`anonymous`),rules:[/^(?:click\b)/i,/^(?:href\b)/i,/^(?:"[^"]*")/i,/^(?:default\b)/i,/^(?:.*direction\s+TB[^\n]*)/i,/^(?:.*direction\s+BT[^\n]*)/i,/^(?:.*direction\s+RL[^\n]*)/i,/^(?:.*direction\s+LR[^\n]*)/i,/^(?:[\n]+)/i,/^(?:[\s]+)/i,/^(?:((?!\n)\s)+)/i,/^(?:#[^\n]*)/i,/^(?:%%(?!\{)[^\n]*)/i,/^(?:scale\s+)/i,/^(?:\d+)/i,/^(?:\s+width\b)/i,/^(?:accTitle\s*:\s*)/i,/^(?:(?!\n||)*[^\n]*)/i,/^(?:accDescr\s*:\s*)/i,/^(?:(?!\n||)*[^\n]*)/i,/^(?:accDescr\s*\{\s*)/i,/^(?:[\}])/i,/^(?:[^\}]*)/i,/^(?:classDef\s+)/i,/^(?:DEFAULT\s+)/i,/^(?:\w+\s+)/i,/^(?:[^\n]*)/i,/^(?:class\s+)/i,/^(?:(\w+)+((,\s*\w+)*))/i,/^(?:[^\n]*)/i,/^(?:style\s+)/i,/^(?:[\w,]+\s+)/i,/^(?:[^\n]*)/i,/^(?:scale\s+)/i,/^(?:\d+)/i,/^(?:\s+width\b)/i,/^(?:state\s+)/i,/^(?:.*<<fork>>)/i,/^(?:.*<<join>>)/i,/^(?:.*<<choice>>)/i,/^(?:.*\[\[fork\]\])/i,/^(?:.*\[\[join\]\])/i,/^(?:.*\[\[choice\]\])/i,/^(?:.*direction\s+TB[^\n]*)/i,/^(?:.*direction\s+BT[^\n]*)/i,/^(?:.*direction\s+RL[^\n]*)/i,/^(?:.*direction\s+LR[^\n]*)/i,/^(?:["])/i,/^(?:\s*as\s+)/i,/^(?:[^\n\{]*)/i,/^(?:["])/i,/^(?:[^"]*)/i,/^(?:\w+\s+\w+.*?\{)/i,/^(?:[^\n\s\{]+)/i,/^(?:\n)/i,/^(?:\{)/i,/^(?:\})/i,/^(?:[\n])/i,/^(?:note\s+)/i,/^(?:left of\b)/i,/^(?:right of\b)/i,/^(?:")/i,/^(?:\s*as\s*)/i,/^(?:["])/i,/^(?:[^"]*)/i,/^(?:[^\n]*)/i,/^(?:\s*[^:\n\s\-]+)/i,/^(?:\s*:[^:\n;]+)/i,/^(?:[\s\S]*?\n\s*end note\b)/i,/^(?:stateDiagram\s+)/i,/^(?:stateDiagram-v2\s+)/i,/^(?:hide empty description\b)/i,/^(?:\[\*\])/i,/^(?:[^:\n\s\-\{]+)/i,/^(?:\s*:(?:[^:\n;]|:[^:\n;])+)/i,/^(?:-->)/i,/^(?:--)/i,/^(?::::)/i,/^(?:$)/i,/^(?:.)/i],conditions:{LINE:{rules:[10,11,12],inclusive:!1},struct:{rules:[10,11,12,23,27,30,36,43,44,45,46,56,57,58,72,73,74,75,76,77],inclusive:!1},FLOATING_NOTE_ID:{rules:[65],inclusive:!1},FLOATING_NOTE:{rules:[62,63,64],inclusive:!1},NOTE_TEXT:{rules:[67,68],inclusive:!1},NOTE_ID:{rules:[66],inclusive:!1},NOTE:{rules:[59,60,61],inclusive:!1},STYLEDEF_STYLEOPTS:{rules:[],inclusive:!1},STYLEDEF_STYLES:{rules:[32],inclusive:!1},STYLE_IDS:{rules:[],inclusive:!1},STYLE:{rules:[31],inclusive:!1},CLASS_STYLE:{rules:[29],inclusive:!1},CLASS:{rules:[28],inclusive:!1},CLASSDEFID:{rules:[26],inclusive:!1},CLASSDEF:{rules:[24,25],inclusive:!1},acc_descr_multiline:{rules:[21,22],inclusive:!1},acc_descr:{rules:[19],inclusive:!1},acc_title:{rules:[17],inclusive:!1},SCALE:{rules:[14,15,34,35],inclusive:!1},ALIAS:{rules:[],inclusive:!1},STATE_ID:{rules:[49],inclusive:!1},STATE_STRING:{rules:[50,51],inclusive:!1},FORK_STATE:{rules:[],inclusive:!1},STATE:{rules:[10,11,12,37,38,39,40,41,42,47,48,52,53,54,55],inclusive:!1},ID:{rules:[10,11,12],inclusive:!1},INITIAL:{rules:[0,1,2,3,4,5,6,7,8,9,11,12,13,16,18,20,23,27,30,33,36,55,58,69,70,71,72,73,74,75,77,78,79],inclusive:!0}}}})();function P(){this.yy={}}return C(P,`Parser`),P.prototype=N,N.Parser=P,new P})();w.parser=w;var T=w,E=`TB`,D=`TB`,O=`dir`,k=`state`,A=`root`,j=`relation`,M=`classDef`,N=`style`,P=`applyClass`,F=`default`,ee=`divider`,te=`fill:none`,ne=`fill: #333`,re=`c`,ie=`markdown`,ae=`normal`,I=`rect`,L=`rectWithTitle`,oe=`stateStart`,se=`stateEnd`,R=`divider`,ce=`roundedWithTitle`,le=`note`,ue=`noteGroup`,z=`statediagram`,de=`${z}-state`,B=`transition`,fe=`note`,pe=`${B} note-edge`,me=`${z}-${fe}`,he=`${z}-cluster`,ge=`${z}-cluster-alt`,V=`parent`,H=`note`,_e=`state`,U=`----`,ve=`${U}${H}`,ye=`${U}${V}`,W=new Map,G=0,be=0,K=new Map,xe=C((e,t,n,r)=>{if(e===R&&n?.id!==void 0&&K.has(n.id)){let e=K.get(n.id);return K.set(t,e),e}let i=be++,a=r?void 0:i;return K.set(t,a),a},`colorSlotFor`);function q(e=``,t=0,n=``,r=U){return`${_e}-${e}${n!==null&&n.length>0?`${r}${n}`:``}-${t}`}C(q,`stateDomId`);var Se=C((e,t,n,r,i,a,o,s)=>{x.trace(`items`,t),t.forEach(t=>{switch(t.stmt){case k:X(e,t,n,r,i,a,o,s);break;case F:X(e,t,n,r,i,a,o,s);break;case j:{X(e,t.state1,n,r,i,a,o,s),X(e,t.state2,n,r,i,a,o,s);let c=o===`neo`,l={id:`edge`+G,start:t.state1.id,end:t.state2.id,arrowhead:`normal`,arrowTypeEnd:c?`arrow_barb_neo`:`arrow_barb`,style:te,labelStyle:``,label:y.sanitizeText(t.description??``,g()),arrowheadStyle:ne,labelpos:re,labelType:ie,thickness:ae,classes:B,look:o};i.push(l),G++}}})},`setupDoc`),Ce=C((e,t=D)=>{let n=t;if(e.doc)for(let t of e.doc)t.stmt===`dir`&&(n=t.value);return n},`getDir`);function J(e,t,n){if(!t.id||t.id===`</join></fork>`||t.id===`</choice>`)return;t.cssClasses&&(Array.isArray(t.cssCompiledStyles)||(t.cssCompiledStyles=[]),t.cssClasses.split(` `).forEach(e=>{let r=n.get(e);r&&(t.cssCompiledStyles=[...t.cssCompiledStyles??[],...r.styles])}));let r=e.find(e=>e.id===t.id);r?Object.assign(r,t):e.push(t)}C(J,`insertOrUpdateNode`);function we(e){return e?.classes?.join(` `)??``}C(we,`getClassesFromDbInfo`);function Y(e){return e?.styles??[]}C(Y,`getStylesFromDbInfo`);var X=C((e,t,n,r,i,a,o,s)=>{let c=t.id,l=n.get(c),u=we(l),d=Y(l),f=g(),p=u.trim()!==``||d.length>0;if(x.info(`dataFetcher parsedItem`,t,l,d),c!==`root`){let n=I;t.start===!0?n=oe:t.start===!1&&(n=se),t.type!==F&&(n=t.type),W.get(c)||W.set(c,{id:c,shape:n,description:y.sanitizeText(c,f),cssClasses:`${u} ${de}`,cssStyles:d});let l=W.get(c);t.description&&(Array.isArray(l.description)?(l.shape=L,l.description.push(t.description)):l.description?.length&&l.description.length>0?(l.shape=L,l.description=l.description===c?[t.description]:[l.description,t.description]):(l.shape=I,l.description=t.description),l.description=y.sanitizeTextOrArray(l.description,f)),l.description?.length===1&&l.shape===L&&(l.shape=l.type===`group`?ce:I),!l.type&&t.doc&&(x.info(`Setting cluster for XCX`,c,Ce(t)),l.type=`group`,l.isGroup=!0,l.dir=Ce(t),l.shape=t.type===ee?R:ce,l.colorIndex=xe(l.shape,c,e,p),l.cssClasses=`${l.cssClasses} ${he} ${a?ge:``}`);let m={labelStyle:``,shape:l.shape,label:l.description,cssClasses:l.cssClasses,cssCompiledStyles:[],cssStyles:l.cssStyles,id:c,dir:l.dir,domId:q(c,G),type:l.type,isGroup:l.type===`group`,colorIndex:l.colorIndex,padding:8,rx:10,ry:10,look:o,labelType:`markdown`};if(m.shape===R&&(m.label=``),e&&e.id!==`root`&&(x.trace(`Setting node `,c,` to be child of its parent `,e.id),m.parentId=e.id),m.centerLabel=!0,t.note){let e={labelStyle:``,shape:le,label:t.note.text,labelType:`markdown`,cssClasses:me,cssStyles:[],cssCompiledStyles:[],id:c+ve+`-`+G,domId:q(c,G,H),type:`node`,isGroup:!1,padding:f.flowchart?.padding,look:o,position:t.note.position},n=c+ye,a={labelStyle:``,shape:ue,label:t.note.text,cssClasses:l.cssClasses,cssStyles:[],id:c+ye,domId:q(c,G,V),type:`group`,isGroup:!0,padding:16,look:o,position:t.note.position};G++,a.id=n,e.parentId=n,J(r,a,s),J(r,e,s),J(r,m,s);let u=c,d=e.id;t.note.position===`left of`&&(u=e.id,d=c),i.push({id:u+`-`+d,start:u,end:d,arrowhead:`none`,arrowTypeEnd:``,style:te,labelStyle:``,classes:pe,pattern:`dashed`,arrowheadStyle:ne,labelpos:re,labelType:ie,thickness:ae,look:o})}else J(r,m,s)}t.doc&&(x.trace(`Adding nodes children `),Se(t,t.doc,n,r,i,!a,o,s))},`dataFetcher`),Te=C(()=>{W.clear(),G=0,be=0,K.clear()},`reset`),Ee=C((e,t=D)=>{if(!e.doc)return t;let n=t;for(let t of e.doc)t.stmt===`dir`&&(n=t.value);return n},`getDir`),De={getClasses:C(function(e,t){return t.db.getClasses()},`getClasses`),draw:C(async function(i,a,o,s){x.info(`REF0:`),x.info(`Drawing state diagram (v2)`,a);let{securityLevel:c,state:u,layout:d}=g();s.db.extract(s.db.getRootDocV2());let f=s.db.getData(),p=e(a,c);f.type=s.type,f.layoutAlgorithm=r(d),f.nodeSpacing=u?.nodeSpacing||50,f.rankSpacing=u?.rankSpacing||50,f.markers=g().look===`neo`?[`barbNeo`]:[`barb`],f.diagramId=a,await n(f,p);try{(typeof s.db.getLinks==`function`?s.db.getLinks():new Map).forEach((e,t)=>{let n=typeof t==`string`?t:typeof t?.id==`string`?t.id:``,r=f.nodes.find(e=>e.id===n);if(!n){x.warn(`⚠️ Invalid or missing stateId from key:`,JSON.stringify(t));return}let i=p.node()?.querySelectorAll(`g.node, g.rough-node`),a;if(i?.forEach(e=>{let t=e.textContent?.trim();(e.id===r?.domId||t===n)&&(a=e)}),!a){x.warn(`⚠️ Could not find node matching text:`,n);return}let o=a.parentNode;if(!o){x.warn(`⚠️ Node has no parent, cannot wrap:`,n);return}let s=document.createElementNS(`http://www.w3.org/2000/svg`,`a`),c=e.url.replace(/^"+|"+$/g,``);if(s.setAttributeNS(`http://www.w3.org/1999/xlink`,`xlink:href`,c),s.setAttribute(`target`,`_blank`),e.tooltip){let t=e.tooltip.replace(/^"+|"+$/g,``);s.setAttribute(`title`,t),a.setAttribute(`title`,t)}o.replaceChild(s,a),s.appendChild(a),x.info(`🔗 Wrapped node in <a> tag for:`,n,e.url)})}catch(e){x.error(`❌ Error injecting clickable links:`,e)}l.insertTitle(p,`statediagramTitleText`,u?.titleTopMargin??25,s.db.getDiagramTitle()),t(p,8,z,u?.useMaxWidth??!0)},`draw`),getDir:Ee},Z={START_NODE:`[*]`,START_TYPE:`start`,END_NODE:`[*]`,END_TYPE:`end`,COLOR_KEYWORD:`color`,FILL_KEYWORD:`fill`,BG_FILL:`bgFill`,STYLECLASS_SEP:`,`},Oe=C(()=>new Map,`newClassesList`),Q=C(()=>({relations:[],states:new Map,documents:{}}),`newDoc`),$=C(e=>JSON.parse(JSON.stringify(e)),`clone`),ke=class{constructor(e){this.version=e,this.nodes=[],this.edges=[],this.rootDoc=[],this.classes=Oe(),this.documents={root:Q()},this.currentDocument=this.documents.root,this.startEndCount=0,this.dividerCnt=0,this.links=new Map,this.funs=[],this.getAccTitle=b,this.setAccTitle=p,this.getAccDescription=m,this.setAccDescription=f,this.setDiagramTitle=v,this.getDiagramTitle=d,this.clear(),this.setRootDoc=this.setRootDoc.bind(this),this.getDividerId=this.getDividerId.bind(this),this.setDirection=this.setDirection.bind(this),this.trimColon=this.trimColon.bind(this),this.bindFunctions=this.bindFunctions.bind(this)}static{C(this,`StateDB`)}static{this.relationType={AGGREGATION:0,EXTENSION:1,COMPOSITION:2,DEPENDENCY:3}}extract(e){this.clear(!0);for(let t of Array.isArray(e)?e:e.doc)switch(t.stmt){case k:this.addState(t.id.trim(),t.type,t.doc,t.description,t.note);break;case j:this.addRelation(t.state1,t.state2,t.description);break;case M:this.addStyleClass(t.id.trim(),t.classes);break;case N:this.handleStyleDef(t);break;case P:this.setCssClass(t.id.trim(),t.styleClass);break;case`click`:this.addLink(t.id,t.url,t.tooltip)}let t=this.getStates(),n=g();Te(),X(void 0,this.getRootDocV2(),t,this.nodes,this.edges,!0,n.look,this.classes);for(let e of this.nodes)if(Array.isArray(e.label)){if(e.description=e.label.slice(1),e.isGroup&&e.description.length>0)throw Error(`Group nodes can only have label. Remove the additional description for node [${e.id}]`);e.label=e.label[0]}}handleStyleDef(e){let t=e.id.trim().split(`,`),n=e.styleClass.split(`,`);for(let e of t){let t=this.getState(e);if(!t){let n=e.trim();this.addState(n),t=this.getState(n)}t&&(t.styles=n.map(e=>e.replace(/;/g,``)?.trim()))}}setRootDoc(e){x.info(`Setting root doc`,e),this.rootDoc=e,this.version===1?this.extract(e):this.extract(this.getRootDocV2())}docTranslator(e,t,n){if(t.stmt===j){this.docTranslator(e,t.state1,!0),this.docTranslator(e,t.state2,!1);return}if(t.stmt===k&&(t.id===Z.START_NODE?(t.id=e.id+(n?`_start`:`_end`),t.start=n):t.id=t.id.trim()),t.stmt!==A&&t.stmt!==k||!t.doc)return;let r=[],i=[];for(let e of t.doc)if(e.type===ee){let t=$(e);t.doc=$(i),r.push(t),i=[]}else i.push(e);if(r.length>0&&i.length>0){let e={stmt:k,id:u(),type:`divider`,doc:$(i)};r.push($(e)),t.doc=r}t.doc.forEach(e=>this.docTranslator(t,e,!0))}getRootDocV2(){return this.docTranslator({id:A,stmt:A},{id:A,stmt:A,doc:this.rootDoc},!0),{id:A,doc:this.rootDoc}}addState(e,t=F,n=void 0,r=void 0,i=void 0,a=void 0,o=void 0,s=void 0){let c=e?.trim();if(!this.currentDocument.states.has(c))x.info(`Adding state `,c,r),this.currentDocument.states.set(c,{stmt:k,id:c,descriptions:[],type:t,doc:n,note:i,classes:[],styles:[],textStyles:[]});else{let e=this.currentDocument.states.get(c);if(!e)throw Error(`State not found: ${c}`);e.doc||=n,e.type||=t}if(r&&(x.info(`Setting state description`,c,r),(Array.isArray(r)?r:[r]).forEach(e=>this.addDescription(c,e.trim()))),i){let e=this.currentDocument.states.get(c);if(!e)throw Error(`State not found: ${c}`);e.note=i,e.note.text=y.sanitizeText(e.note.text,g())}a&&(x.info(`Setting state classes`,c,a),(Array.isArray(a)?a:[a]).forEach(e=>this.setCssClass(c,e.trim()))),o&&(x.info(`Setting state styles`,c,o),(Array.isArray(o)?o:[o]).forEach(e=>this.setStyle(c,e.trim()))),s&&(x.info(`Setting state styles`,c,o),(Array.isArray(s)?s:[s]).forEach(e=>this.setTextStyle(c,e.trim())))}clear(e){this.nodes=[],this.edges=[],this.funs=[this.setupToolTips.bind(this)],this.documents={root:Q()},this.currentDocument=this.documents.root,this.startEndCount=0,this.classes=Oe(),e||(this.links=new Map,h())}getState(e){return this.currentDocument.states.get(e)}getStates(){return this.currentDocument.states}logDocuments(){x.info(`Documents = `,this.documents)}getRelations(){return this.currentDocument.relations}addLink(e,t,n){this.links.set(e,{url:t,tooltip:n}),x.warn(`Adding link`,e,t,n)}getLinks(){return this.links}startIdIfNeeded(e=``){return e===Z.START_NODE?(this.startEndCount++,`${Z.START_TYPE}${this.startEndCount}`):e}startTypeIfNeeded(e=``,t=F){return e===Z.START_NODE?Z.START_TYPE:t}endIdIfNeeded(e=``){return e===Z.END_NODE?(this.startEndCount++,`${Z.END_TYPE}${this.startEndCount}`):e}endTypeIfNeeded(e=``,t=F){return e===Z.END_NODE?Z.END_TYPE:t}addRelationObjs(e,t,n=``){let r=this.startIdIfNeeded(e.id.trim()),i=this.startTypeIfNeeded(e.id.trim(),e.type),a=this.startIdIfNeeded(t.id.trim()),o=this.startTypeIfNeeded(t.id.trim(),t.type);this.addState(r,i,e.doc,e.description,e.note,e.classes,e.styles,e.textStyles),this.addState(a,o,t.doc,t.description,t.note,t.classes,t.styles,t.textStyles),this.currentDocument.relations.push({id1:r,id2:a,relationTitle:y.sanitizeText(n,g())})}addRelation(e,t,n){if(typeof e==`object`&&typeof t==`object`)this.addRelationObjs(e,t,n);else if(typeof e==`string`&&typeof t==`string`){let r=this.startIdIfNeeded(e.trim()),i=this.startTypeIfNeeded(e),a=this.endIdIfNeeded(t.trim()),o=this.endTypeIfNeeded(t);this.addState(r,i),this.addState(a,o),this.currentDocument.relations.push({id1:r,id2:a,relationTitle:n?y.sanitizeText(n,g()):void 0})}}addDescription(e,t){let n=this.currentDocument.states.get(e),r=t.startsWith(`:`)?t.replace(`:`,``).trim():t;n?.descriptions?.push(y.sanitizeText(r,g()))}cleanupLabel(e){return e.startsWith(`:`)?e.slice(2).trim():e.trim()}getDividerId(){return this.dividerCnt++,`divider-id-${this.dividerCnt}`}addStyleClass(e,t=``){this.classes.has(e)||this.classes.set(e,{id:e,styles:[],textStyles:[]});let n=this.classes.get(e);t&&n&&t.split(Z.STYLECLASS_SEP).forEach(e=>{let t=e.replace(/([^;]*);/,`$1`).trim();if(RegExp(Z.COLOR_KEYWORD).exec(e)){let e=t.replace(Z.FILL_KEYWORD,Z.BG_FILL).replace(Z.COLOR_KEYWORD,Z.FILL_KEYWORD);n.textStyles.push(e)}n.styles.push(t)})}getClasses(){return this.classes}setupToolTips(e){let t=i();S(e).select(`svg`).selectAll(`g.node, g.rough-node`).on(`mouseover`,e=>{let n=S(e.currentTarget),r=n.attr(`title`);if(r===null)return;let i=e.currentTarget?.getBoundingClientRect();t.transition().duration(200).style(`opacity`,`.9`),t.style(`left`,window.scrollX+i.left+(i.right-i.left)/2+`px`).style(`top`,window.scrollY+i.bottom+`px`),t.html(_.sanitize(r)),n.classed(`hover`,!0)}).on(`mouseout`,e=>{t.transition().duration(500).style(`opacity`,0),S(e.currentTarget).classed(`hover`,!1)})}setCssClass(e,t){e.split(`,`).forEach(e=>{let n=this.getState(e);if(!n){let t=e.trim();this.addState(t),n=this.getState(t)}n?.classes?.push(t)})}setStyle(e,t){this.getState(e)?.styles?.push(t)}setTextStyle(e,t){this.getState(e)?.textStyles?.push(t)}bindFunctions(e){this.funs.forEach(t=>{t(e)})}getDirectionStatement(){return this.rootDoc.find(e=>e.stmt===O)}getDirection(){return this.getDirectionStatement()?.value??E}setDirection(e){let t=this.getDirectionStatement();t?t.value=e:this.rootDoc.unshift({stmt:O,value:e})}trimColon(e){return e.startsWith(`:`)?e.slice(1).trim():e.trim()}getData(){let e=g();for(let t of this.nodes)t.wrappingWidth??=e.state?.wrappingWidth,t.isGroup||(t.minWidth??=e.state?.minNodeWidth);return{nodes:this.nodes,edges:this.edges,other:{},config:e,direction:Ee(this.getRootDocV2())}}getConfig(){return g().state}},Ae=C(e=>{let{theme:t,bkgColorArray:n,borderColorArray:r}=e;if(!o(t,r))return``;let i=s(e.look),l=c(n),u=``;for(let e=0;e<a(r);e++){let t=r[e],a=l?`fill: ${n[e%n.length]};`:``,o=`[data-look="${i}"][data-color-id="color-${e}"]`;u+=`

    /* The title strip: \`rect.outer\` spans the whole composite and \`rect.inner\` covers
       the body, so what stays visible of \`outer\` is the band behind the label. */
    ${o}.statediagram-cluster rect.outer {
      stroke: ${t};
      ${a}
    }

    ${o}.statediagram-cluster rect.inner {
      stroke: ${t};
    }

    /* Concurrency regions. Siblings of one composite share a slot, so a divided composite
       reads as one thing split into parts rather than as several composites. */
    ${o}.statediagram-cluster rect.divider {
      stroke: ${t};
      ${a}
    }

    /* handDrawn draws the same container as roughjs shapes rather than plain rects, so it
       needs its own rules. \`roundedWithTitle\` and \`divider\` name those groups \`outer\`,
       \`inner\` and \`divider\` to match the classic branch, which is what lets these
       discriminate -- a bare \`.statediagram-cluster path\` rule reached the body as well and
       tinted the whole composite, losing \`compositeBackground\` and diverging from what
       classic and neo do.

       roughjs emits two paths per shape and marks them: the filled shape carries
       \`stroke="none"\` and the sketched outline carries \`fill="none"\`. Splitting on that is
       what keeps \`fill\` off the outline -- a rough outline is open squiggles, not a closed
       region, so filling it produces smears -- and keeps \`stroke\` off the fill shape, which
       would otherwise gain an edge it was drawn without. */
    ${o}.statediagram-cluster .outer path[stroke='none'] {
      ${a}
    }

    ${o}.statediagram-cluster .outer path[fill='none'] {
      stroke: ${t};
    }

    /* No \`.inner\` rule on purpose. The body shape is left entirely alone under handDrawn,
       where a rect's \`inner\` counterpart cannot be recoloured safely: roughjs draws a
       hachure fill as *stroked* lines, so its fill paths carry \`fill="none"\` exactly like
       the outline and no selector separates them. An \`.inner\` stroke rule therefore
       repainted the hatching of every alt composite in the palette colour instead of
       leaving it on \`altBackground\`. The container still reads as palette-coloured: the
       \`outer\` shape spans the whole composite, so its outline already frames the body. */

    /* Regions split the same way, which is why \`divider\` fills solid rather than taking
       roughjs's default hachure -- see the note on that call. Hatched, both of its paths
       carried \`fill="none"\` and these two rules degenerated: the tint matched nothing and
       the border rule repainted the hatching. */
    ${o}.statediagram-cluster .divider path[stroke='none'] {
      ${a}
    }

    ${o}.statediagram-cluster .divider path[fill='none'] {
      stroke: ${t};
    }
    `}return u},`genColor`),je={parser:T,get db(){return new ke(2)},renderer:De,styles:C(e=>`
${Ae(e)}
defs [id$="-barbEnd"] {
    fill: ${e.transitionColor};
    stroke: ${e.transitionColor};
  }
g.stateGroup text {
  fill: ${e.nodeBorder};
  stroke: none;
  font-size: 10px;
}
g.stateGroup text {
  fill: ${e.textColor};
  stroke: none;
  font-size: 10px;

}
g.stateGroup .state-title {
  font-weight: bolder;
  fill: ${e.stateLabelColor};
}

g.stateGroup rect {
  fill: ${e.mainBkg};
  stroke: ${e.nodeBorder};
}

g.stateGroup line {
  stroke: ${e.lineColor};
  stroke-width: ${e.strokeWidth||1};
}

.transition {
  stroke: ${e.transitionColor};
  stroke-width: ${e.strokeWidth||1};
  fill: none;
}

.stateGroup .composit {
  fill: ${e.background};
  border-bottom: 1px
}

.stateGroup .alt-composit {
  fill: #e0e0e0;
  border-bottom: 1px
}

.state-note {
  stroke: ${e.noteBorderColor};
  fill: ${e.noteBkgColor};

  text {
    fill: ${e.noteTextColor};
    stroke: none;
    font-size: 10px;
  }
}

.stateLabel .box {
  stroke: none;
  stroke-width: 0;
  fill: ${e.mainBkg};
  opacity: 0.5;
}

.edgeLabel .label rect {
  fill: ${e.labelBackgroundColor};
  opacity: 0.5;
}
.edgeLabel {
  background-color: ${e.edgeLabelBackground};
  p {
    background-color: ${e.edgeLabelBackground};
  }
  rect {
    opacity: 0.5;
    background-color: ${e.edgeLabelBackground};
    fill: ${e.edgeLabelBackground};
  }
  text-align: center;
}
.edgeLabel .label text {
  fill: ${e.transitionLabelColor||e.tertiaryTextColor};
}
.label div .edgeLabel {
  color: ${e.transitionLabelColor||e.tertiaryTextColor};
}

.stateLabel text {
  fill: ${e.stateLabelColor};
  font-size: 10px;
  font-weight: bold;
}

.node circle.state-start {
  fill: ${e.specialStateColor};
  stroke: ${e.specialStateColor};
}

.node .fork-join {
  fill: ${e.specialStateColor};
  stroke: ${e.specialStateColor};
}

.node circle.state-end {
  fill: ${e.innerEndBackground};
  stroke: ${e.background};
  stroke-width: 1.5
}
.end-state-inner {
  fill: ${e.compositeBackground||e.background};
  // stroke: ${e.background};
  stroke-width: 1.5
}

.node rect {
  fill: ${e.stateBkg||e.mainBkg};
  stroke: ${e.stateBorder||e.nodeBorder};
  stroke-width: ${e.strokeWidth||1}px;
}
.node polygon {
  fill: ${e.mainBkg};
  stroke: ${e.stateBorder||e.nodeBorder};;
  stroke-width: ${e.strokeWidth||1}px;
}
[id$="-barbEnd"] {
  fill: ${e.lineColor};
}

.statediagram-cluster rect {
  fill: ${e.compositeTitleBackground};
  stroke: ${e.stateBorder||e.nodeBorder};
  stroke-width: ${e.strokeWidth||1}px;
}

.cluster-label, .nodeLabel {
  color: ${e.stateLabelColor};
  // line-height: 1;
}

.statediagram-cluster rect.outer {
  rx: 5px;
  ry: 5px;
}
.statediagram-state .divider {
  stroke: ${e.stateBorder||e.nodeBorder};
}

.statediagram-state .title-state {
  rx: 5px;
  ry: 5px;
}
.statediagram-cluster.statediagram-cluster .inner {
  fill: ${e.compositeBackground||e.background};
}
.statediagram-cluster.statediagram-cluster-alt .inner {
  fill: ${e.altBackground?e.altBackground:`#efefef`};
}

.statediagram-cluster .inner {
  rx:0;
  ry:0;
}

.statediagram-state rect.basic {
  rx: 5px;
  ry: 5px;
}
.statediagram-state rect.divider {
  stroke-dasharray: 10,10;
  fill: ${e.altBackground?e.altBackground:`#efefef`};
}

.note-edge {
  stroke-dasharray: 5;
}

.statediagram-note rect {
  fill: ${e.noteBkgColor};
  stroke: ${e.noteBorderColor};
  stroke-width: 1px;
  rx: 0;
  ry: 0;
}
.statediagram-note rect {
  fill: ${e.noteBkgColor};
  stroke: ${e.noteBorderColor};
  stroke-width: 1px;
  rx: 0;
  ry: 0;
}

.statediagram-note text {
  fill: ${e.noteTextColor};
}

.statediagram-note .nodeLabel {
  color: ${e.noteTextColor};
}
.statediagram .edgeLabel {
  color: red; // ${e.noteTextColor};
}

[id$="-dependencyStart"], [id$="-dependencyEnd"] {
  fill: ${e.lineColor};
  stroke: ${e.lineColor};
  stroke-width: ${e.strokeWidth||1};
}

.statediagramTitleText {
  text-anchor: middle;
  font-size: 18px;
  fill: ${e.textColor};
}

[data-look="neo"].statediagram-cluster rect {
  fill: ${e.mainBkg};
  stroke: ${e.useGradient?`url(`+e.svgId+`-gradient)`:e.stateBorder||e.nodeBorder};
  stroke-width: ${e.strokeWidth??1};
}
[data-look="neo"].statediagram-cluster rect.outer {
  rx: ${e.radius}px;
  ry: ${e.radius}px;
  filter: ${e.dropShadow?e.dropShadow.replace(`url(#drop-shadow)`,`url(${e.svgId}-drop-shadow)`):`none`}
}
`,`getStyles`),init:C(e=>{e.state||={},e.state.arrowMarkerAbsolute=e.arrowMarkerAbsolute},`init`)};export{je as diagram};