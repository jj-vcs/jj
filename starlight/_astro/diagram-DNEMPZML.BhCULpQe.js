import{t as e}from"./chunk-JWPE2WC7.A3RzWITa.js";import{a as t}from"./mermaid.core.Bf0pvaT3.js";import{i as n}from"./chunk-6MUBBAK2.CfXUfaG5.js";import{C as r,H as i,U as a,_ as o,a as s,c,f as l,q as u,v as d,y as f}from"./chunk-BG4553SI.DzMjcvML.js";import{m as p}from"./src.Duoub3vK.js";import{n as m}from"./chunk-Y2CYZVJY.DsF7k-Jl.js";import{n as h}from"./mermaid-parser.core.CZlxp23a.js";var g=l.packet,_=class{constructor(){this.packet=[],this.setAccTitle=a,this.getAccTitle=d,this.setDiagramTitle=u,this.getDiagramTitle=r,this.getAccDescription=o,this.setAccDescription=i}static{m(this,`PacketDB`)}getConfig(){let e=n({...g,...f().packet});return e.showBits&&(e.paddingY+=10),e}getPacket(){return this.packet}pushWord(e){e.length>0&&this.packet.push(e)}clear(){s(),this.packet=[]}},v=1e4,y=m((t,n)=>{e(t,n);let r=-1,i=[],a=1,{bitsPerRow:o}=n.getConfig();for(let{start:e,end:s,bits:c,label:l}of t.blocks){if(e!==void 0&&s!==void 0&&s<e)throw Error(`Packet block ${e} - ${s} is invalid. End must be greater than start.`);if(e??=r+1,e!==r+1)throw Error(`Packet block ${e} - ${s??e} is not contiguous. It should start from ${r+1}.`);if(c===0)throw Error(`Packet block ${e} is invalid. Cannot have a zero bit field.`);for(s??=e+(c??1)-1,c??=s-e+1,r=s,p.debug(`Packet block ${e} - ${r} with label ${l}`);i.length<=o+1&&n.getPacket().length<v;){let[t,r]=b({start:e,end:s,bits:c,label:l},a,o);if(i.push(t),t.end+1===a*o&&(n.pushWord(i),i=[],a++),!r)break;({start:e,end:s,bits:c,label:l}=r)}}n.pushWord(i)},`populate`),b=m((e,t,n)=>{if(e.start===void 0)throw Error(`start should have been set during first phase`);if(e.end===void 0)throw Error(`end should have been set during first phase`);if(e.start>e.end)throw Error(`Block start ${e.start} is greater than block end ${e.end}.`);if(e.end+1<=t*n)return[e,void 0];let r=t*n-1,i=t*n;return[{start:e.start,end:r,label:e.label,bits:r-e.start},{start:i,end:e.end,label:e.label,bits:e.end-i}]},`getNextFittingBlock`),x={parser:{yy:void 0},parse:m(async e=>{let t=await h(`packet`,e),n=x.parser?.yy;if(!(n instanceof _))throw Error(`parser.parser?.yy was not a PacketDB. This is due to a bug within Mermaid, please report this issue at https://github.com/mermaid-js/mermaid/issues.`);p.debug(t),y(t,n)},`parse`)},S=m((e,n,r,i)=>{let a=i.db,o=a.getConfig(),{rowHeight:s,paddingY:l,bitWidth:u,bitsPerRow:d}=o,f=a.getPacket(),p=a.getDiagramTitle(),m=s+l,h=m*(f.length+1)-(p?0:s),g=u*d+2,_=t(n);_.attr(`viewBox`,`0 0 ${g} ${h}`),c(_,h,g,o.useMaxWidth);for(let[e,t]of f.entries())C(_,t,e,o);_.append(`text`).text(p).attr(`x`,g/2).attr(`y`,h-m/2).attr(`dominant-baseline`,`middle`).attr(`text-anchor`,`middle`).attr(`class`,`packetTitle`)},`draw`),C=m((e,t,n,{rowHeight:r,paddingX:i,paddingY:a,bitWidth:o,bitsPerRow:s,showBits:c,bitOrder:l})=>{let u=e.append(`g`),d=n*(r+a)+a,f=l===`descending`;for(let e of t){let t=e.end-e.start+1,n=e.start%s,a=(f?s-n-t:n)*o+1,l=t*o-i;if(u.append(`rect`).attr(`x`,a).attr(`y`,d).attr(`width`,l).attr(`height`,r).attr(`class`,`packetBlock`),u.append(`text`).attr(`x`,a+l/2).attr(`y`,d+r/2).attr(`class`,`packetLabel`).attr(`dominant-baseline`,`middle`).attr(`text-anchor`,`middle`).text(e.label),!c)continue;let[p,m]=f?[e.end,e.start]:[e.start,e.end],h=t===1,g=d-2;u.append(`text`).attr(`x`,a+(h?l/2:0)).attr(`y`,g).attr(`class`,`packetByte start`).attr(`dominant-baseline`,`auto`).attr(`text-anchor`,h?`middle`:`start`).text(p),h||u.append(`text`).attr(`x`,a+l).attr(`y`,g).attr(`class`,`packetByte end`).attr(`dominant-baseline`,`auto`).attr(`text-anchor`,`end`).text(m)}},`drawWord`),w={draw:S},T={byteFontSize:`10px`,startByteColor:`black`,endByteColor:`black`,labelColor:`black`,labelFontSize:`12px`,titleColor:`black`,titleFontSize:`14px`,blockStrokeColor:`black`,blockStrokeWidth:`1`,blockFillColor:`#efefef`},E={parser:x,get db(){return new _},renderer:w,styles:m(({packet:e}={})=>{let t=n(T,e);return`
	.packetByte {
		font-size: ${t.byteFontSize};
	}
	.packetByte.start {
		fill: ${t.startByteColor};
	}
	.packetByte.end {
		fill: ${t.endByteColor};
	}
	.packetLabel {
		fill: ${t.labelColor};
		font-size: ${t.labelFontSize};
	}
	.packetTitle {
		fill: ${t.titleColor};
		font-size: ${t.titleFontSize};
	}
	.packetBlock {
		stroke: ${t.blockStrokeColor};
		stroke-width: ${t.blockStrokeWidth};
		fill: ${t.blockFillColor};
	}
	`},`styles`)};export{E as diagram};