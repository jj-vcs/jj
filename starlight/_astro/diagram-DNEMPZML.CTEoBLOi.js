import{m as e}from"./src._wTxv9u9.js";import{n as t}from"./mermaid-parser.core.23u7-xAn.js";import{n}from"./chunk-Y2CYZVJY.DsF7k-Jl.js";import{C as r,H as i,U as a,_ as o,a as s,c,f as l,q as u,v as d,y as f}from"./chunk-BG4553SI.tiJAX5Ut.js";import{i as p}from"./chunk-6MUBBAK2.OBS9Jdm-.js";import{t as m}from"./chunk-JWPE2WC7.A3RzWITa.js";import{a as h}from"./mermaid.core.DGyw8QGY.js";var g=l.packet,_=class{constructor(){this.packet=[],this.setAccTitle=a,this.getAccTitle=d,this.setDiagramTitle=u,this.getDiagramTitle=r,this.getAccDescription=o,this.setAccDescription=i}static{n(this,`PacketDB`)}getConfig(){let e=p({...g,...f().packet});return e.showBits&&(e.paddingY+=10),e}getPacket(){return this.packet}pushWord(e){e.length>0&&this.packet.push(e)}clear(){s(),this.packet=[]}},v=1e4,y=n((t,n)=>{m(t,n);let r=-1,i=[],a=1,{bitsPerRow:o}=n.getConfig();for(let{start:s,end:c,bits:l,label:u}of t.blocks){if(s!==void 0&&c!==void 0&&c<s)throw Error(`Packet block ${s} - ${c} is invalid. End must be greater than start.`);if(s??=r+1,s!==r+1)throw Error(`Packet block ${s} - ${c??s} is not contiguous. It should start from ${r+1}.`);if(l===0)throw Error(`Packet block ${s} is invalid. Cannot have a zero bit field.`);for(c??=s+(l??1)-1,l??=c-s+1,r=c,e.debug(`Packet block ${s} - ${r} with label ${u}`);i.length<=o+1&&n.getPacket().length<v;){let[e,t]=b({start:s,end:c,bits:l,label:u},a,o);if(i.push(e),e.end+1===a*o&&(n.pushWord(i),i=[],a++),!t)break;({start:s,end:c,bits:l,label:u}=t)}}n.pushWord(i)},`populate`),b=n((e,t,n)=>{if(e.start===void 0)throw Error(`start should have been set during first phase`);if(e.end===void 0)throw Error(`end should have been set during first phase`);if(e.start>e.end)throw Error(`Block start ${e.start} is greater than block end ${e.end}.`);if(e.end+1<=t*n)return[e,void 0];let r=t*n-1,i=t*n;return[{start:e.start,end:r,label:e.label,bits:r-e.start},{start:i,end:e.end,label:e.label,bits:e.end-i}]},`getNextFittingBlock`),x={parser:{yy:void 0},parse:n(async n=>{let r=await t(`packet`,n),i=x.parser?.yy;if(!(i instanceof _))throw Error(`parser.parser?.yy was not a PacketDB. This is due to a bug within Mermaid, please report this issue at https://github.com/mermaid-js/mermaid/issues.`);e.debug(r),y(r,i)},`parse`)},S=n((e,t,n,r)=>{let i=r.db,a=i.getConfig(),{rowHeight:o,paddingY:s,bitWidth:l,bitsPerRow:u}=a,d=i.getPacket(),f=i.getDiagramTitle(),p=o+s,m=p*(d.length+1)-(f?0:o),g=l*u+2,_=h(t);_.attr(`viewBox`,`0 0 ${g} ${m}`),c(_,m,g,a.useMaxWidth);for(let[e,t]of d.entries())C(_,t,e,a);_.append(`text`).text(f).attr(`x`,g/2).attr(`y`,m-p/2).attr(`dominant-baseline`,`middle`).attr(`text-anchor`,`middle`).attr(`class`,`packetTitle`)},`draw`),C=n((e,t,n,{rowHeight:r,paddingX:i,paddingY:a,bitWidth:o,bitsPerRow:s,showBits:c,bitOrder:l})=>{let u=e.append(`g`),d=n*(r+a)+a,f=l===`descending`;for(let e of t){let t=e.end-e.start+1,n=e.start%s,a=(f?s-n-t:n)*o+1,l=t*o-i;if(u.append(`rect`).attr(`x`,a).attr(`y`,d).attr(`width`,l).attr(`height`,r).attr(`class`,`packetBlock`),u.append(`text`).attr(`x`,a+l/2).attr(`y`,d+r/2).attr(`class`,`packetLabel`).attr(`dominant-baseline`,`middle`).attr(`text-anchor`,`middle`).text(e.label),!c)continue;let[p,m]=f?[e.end,e.start]:[e.start,e.end],h=t===1,g=d-2;u.append(`text`).attr(`x`,a+(h?l/2:0)).attr(`y`,g).attr(`class`,`packetByte start`).attr(`dominant-baseline`,`auto`).attr(`text-anchor`,h?`middle`:`start`).text(p),h||u.append(`text`).attr(`x`,a+l).attr(`y`,g).attr(`class`,`packetByte end`).attr(`dominant-baseline`,`auto`).attr(`text-anchor`,`end`).text(m)}},`drawWord`),w={draw:S},T={byteFontSize:`10px`,startByteColor:`black`,endByteColor:`black`,labelColor:`black`,labelFontSize:`12px`,titleColor:`black`,titleFontSize:`14px`,blockStrokeColor:`black`,blockStrokeWidth:`1`,blockFillColor:`#efefef`},E={parser:x,get db(){return new _},renderer:w,styles:n(({packet:e}={})=>{let t=p(T,e);return`
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